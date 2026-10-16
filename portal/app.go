package main

import (
	"context"
	"crypto/md5"
	"crypto/subtle"
	"encoding/hex"
	"errors"
	"html/template"
	"io/fs"
	"log"
	"net"
	"net/http"
	"net/mail"
	"regexp"
	"strconv"
	"strings"
	"time"
)

type app struct {
	store      *store
	sessions   *sessions
	pages      map[string]*template.Template
	adminGroup int
	conn       connectInfo
	// The client patcher's own window, outside the site's layout.
	patcherPageTmpl *template.Template
	loginFails      *limiter // failed logins per IP
	signups         *limiter // new accounts per IP
	passwordTries   *limiter // wrong current passwords per account
}

// dbTimeout bounds each page's database work. Requests are only cancelled
// when the browser goes away, so without it a stalled database (a backup
// lock, a long restore) would pile requests up behind the connection pool.
const dbTimeout = 5 * time.Second

// groups are the account groups in rAthena's stock conf/groups.yml.
var groups = []struct {
	ID   int
	Name string
}{
	{0, "Player"},
	{1, "Super Player"},
	{2, "Support"},
	{3, "Script Manager"},
	{4, "Event Manager"},
	{5, "VIP"},
	{10, "Law Enforcement"},
	{99, "Admin"},
}

func groupName(id int) string {
	for _, g := range groups {
		if g.ID == id {
			return g.Name
		}
	}
	return "Group " + strconv.Itoa(id)
}

func newApp(st *store, adminGroup int, conn connectInfo) (*app, error) {
	funcs := template.FuncMap{
		"groupName":     groupName,
		"groups":        func() any { return groups },
		"date":          func(t time.Time) string { return t.Format("2006-01-02 15:04 MST") },
		"town":          func() string { return townMap + " (" + strconv.Itoa(townX) + ", " + strconv.Itoa(townY) + ")" },
		"jobName":       jobName,
		"inc":           func(i int) int { return i + 1 },
		"thousands":     thousands,
		"rankingLimit":  func() int { return rankingLimit },
		"lookupMissing": func() string { return lookupMissingText },
	}
	pages := map[string]*template.Template{}
	for _, name := range []string{"login", "register", "account", "admin", "admin_account", "admin_character", "connect", "online", "rankings", "market", "news", "error",
		"lookup_items", "lookup_item", "lookup_monsters", "lookup_monster"} {
		t, err := template.New("").Funcs(funcs).ParseFS(assets, "templates/layout.html", "templates/items.html", "templates/lookup.html", "templates/"+name+".html")
		if err != nil {
			return nil, err
		}
		pages[name] = t
	}
	patcherPage, err := parsePatcherPage()
	if err != nil {
		return nil, err
	}
	a := &app{
		patcherPageTmpl: patcherPage,
		store:           st,
		sessions:        newSessions(),
		pages:           pages,
		adminGroup:      adminGroup,
		conn:            conn,
		loginFails:      newLimiter(10, 15*time.Minute),
		signups:         newLimiter(5, time.Hour),
		passwordTries:   newLimiter(10, 15*time.Minute),
	}
	go func() {
		for range time.Tick(10 * time.Minute) {
			a.loginFails.prune()
			a.signups.prune()
			a.passwordTries.prune()
		}
	}()
	return a, nil
}

func (a *app) routes() http.Handler {
	static, _ := fs.Sub(assets, "static")
	mux := http.NewServeMux()
	mux.Handle("GET /static/", http.StripPrefix("/static/", http.FileServerFS(static)))
	mux.HandleFunc("GET /healthz", a.healthz)
	mux.HandleFunc("GET /{$}", a.page(a.home))
	mux.HandleFunc("GET /connect", a.page(a.connect))
	mux.HandleFunc("GET /online", a.page(a.online))
	mux.HandleFunc("GET /rankings", a.page(a.rankings))
	mux.HandleFunc("GET /market", a.page(a.market))
	mux.HandleFunc("GET /news", a.page(a.newsPage))
	mux.HandleFunc("GET /items", a.page(a.itemsPage))
	mux.HandleFunc("GET /items/{id}", a.page(a.itemPage))
	mux.HandleFunc("GET /monsters", a.page(a.monstersPage))
	mux.HandleFunc("GET /monsters/{id}", a.page(a.monsterPage))
	mux.HandleFunc("GET /connect/clientinfo.xml", a.clientInfoFile)
	mux.HandleFunc("GET /connect/ExternalSettings_kr.lub", a.externalSettingsFile)
	mux.HandleFunc("GET /connect/"+linuxLauncherName, a.linuxLauncherFile)
	mux.HandleFunc("GET /connect/"+patcherZip, a.patcherDownload)
	mux.HandleFunc("GET /connect/"+newPatcherZip, a.newPatcherDownload)
	mux.HandleFunc("GET /patcher/info.json", a.patcherInfo)
	mux.HandleFunc("GET /patcher/"+patcherName+".exe", a.newPatcherExe)
	mux.HandleFunc("GET /patcher/{$}", a.patcherPage)
	mux.HandleFunc("GET /patcher/plist.txt", a.patcherList)
	mux.HandleFunc("GET /patcher/data/{name}", a.patchFile)
	mux.HandleFunc("GET /login", a.page(a.loginForm))
	mux.HandleFunc("POST /login", a.page(a.login))
	mux.HandleFunc("POST /logout", a.page(a.logout))
	mux.HandleFunc("GET /register", a.page(a.registerForm))
	mux.HandleFunc("POST /register", a.page(a.register))
	mux.HandleFunc("GET /account", a.user(a.account))
	mux.HandleFunc("POST /account/password", a.user(a.changePassword))
	mux.HandleFunc("POST /account/details", a.user(a.changeDetails))
	mux.HandleFunc("GET /admin", a.admin(a.adminList))
	mux.HandleFunc("GET /admin/accounts/{id}", a.admin(a.adminAccount))
	mux.HandleFunc("POST /admin/accounts/{id}/group", a.admin(a.adminSetGroup))
	mux.HandleFunc("POST /admin/accounts/{id}/ban", a.admin(a.adminBan))
	mux.HandleFunc("POST /admin/accounts/{id}/unban", a.admin(a.adminUnban))
	mux.HandleFunc("POST /admin/accounts/{id}/password", a.admin(a.adminSetPassword))
	mux.HandleFunc("GET /admin/characters/{id}", a.admin(a.adminCharacter))
	mux.HandleFunc("POST /admin/characters/{id}/save-point", a.admin(a.adminMoveToSavePoint))
	mux.HandleFunc("POST /admin/characters/{id}/town", a.admin(a.adminMoveToTown))
	return securityHeaders(mux)
}

func securityHeaders(next http.Handler) http.Handler {
	return http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		h := w.Header()
		h.Set("Content-Security-Policy", "default-src 'self'; frame-ancestors 'none'; form-action 'self'")
		h.Set("X-Content-Type-Options", "nosniff")
		h.Set("X-Frame-Options", "DENY")
		h.Set("Referrer-Policy", "same-origin")
		next.ServeHTTP(w, r)
	})
}

// request is what every page handler gets: the session and, once logged
// in, the account as it is in the database right now.
type request struct {
	w       http.ResponseWriter
	r       *http.Request
	sess    *session
	account *Account
}

// view is what every template gets.
type view struct {
	Title   string
	Account *Account
	IsAdmin bool
	CSRF    string
	Flash   string
	Error   string
	Form    map[string]string
	Data    any
}

// page wraps a handler with the session, CSRF check and account lookup.
func (a *app) page(h func(*request) error) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		ctx, cancel := context.WithTimeout(r.Context(), dbTimeout)
		defer cancel()
		r = r.WithContext(ctx)
		req := &request{w: w, r: r, sess: a.sessions.get(r)}
		if r.Method == http.MethodPost {
			r.Body = http.MaxBytesReader(w, r.Body, 64<<10)
			if err := r.ParseForm(); err != nil {
				http.Error(w, "bad form", http.StatusBadRequest)
				return
			}
			// A visitor's blank session has no token, and an empty form
			// value mustn't match it.
			if !req.sess.kept() || subtle.ConstantTimeCompare([]byte(r.PostFormValue("csrf")), []byte(req.sess.csrf)) != 1 {
				a.render(req, http.StatusForbidden, "error", view{Title: "Expired form",
					Error: "That form had expired. Go back, reload the page and try again."})
				return
			}
		}
		if req.sess.accountID != 0 {
			acc, err := a.store.accountByID(r.Context(), req.sess.accountID)
			if err != nil {
				a.fail(req, err)
				return
			}
			if acc == nil || acc.Blocked() || acc.Banned() {
				// Deleted or banned since logging in.
				a.sessions.end(w, req.sess)
				http.Redirect(w, r, "/login", http.StatusSeeOther)
				return
			}
			req.account = acc
		}
		if err := h(req); err != nil {
			a.fail(req, err)
		}
	}
}

func (a *app) user(h func(*request) error) http.HandlerFunc {
	return a.page(func(req *request) error {
		if req.account == nil {
			http.Redirect(req.w, req.r, "/login", http.StatusSeeOther)
			return nil
		}
		return h(req)
	})
}

func (a *app) admin(h func(*request) error) http.HandlerFunc {
	return a.user(func(req *request) error {
		if req.account.GroupID != a.adminGroup {
			a.render(req, http.StatusForbidden, "error", view{Title: "Not allowed", Error: "Only admins can open that page."})
			return nil
		}
		return h(req)
	})
}

func (a *app) render(req *request, status int, name string, v view) {
	v.Account = req.account
	v.IsAdmin = req.account != nil && req.account.GroupID == a.adminGroup
	v.CSRF = req.sess.csrf
	if v.Flash == "" {
		v.Flash = a.sessions.takeFlash(req.sess)
	}
	req.w.Header().Set("Content-Type", "text/html; charset=utf-8")
	req.w.Header().Set("Cache-Control", "no-store")
	req.w.WriteHeader(status)
	if err := a.pages[name].ExecuteTemplate(req.w, "layout", v); err != nil {
		log.Printf("render %s: %v", name, err)
	}
}

func (a *app) fail(req *request, err error) {
	log.Printf("%s %s: %v", req.r.Method, req.r.URL.Path, err)
	a.render(req, http.StatusInternalServerError, "error", view{Title: "Something went wrong",
		Error: "The portal couldn't reach the game database. Try again in a moment."})
}

// keepSession stores a session for a visitor who doesn't have one yet,
// for a page with a form (its CSRF token) or a message for the next page.
func (a *app) keepSession(req *request) {
	if !req.sess.kept() {
		req.sess = a.sessions.start(req.w, 0, anonTTL)
	}
}

// redirect sends the browser on after a successful POST, with a message
// for the next page.
func (a *app) redirect(req *request, to, flash string) error {
	if flash != "" {
		a.keepSession(req)
		a.sessions.setFlash(req.sess, flash)
	}
	http.Redirect(req.w, req.r, to, http.StatusSeeOther)
	return nil
}

func (a *app) healthz(w http.ResponseWriter, r *http.Request) {
	ctx, cancel := context.WithTimeout(r.Context(), 2*time.Second)
	defer cancel()
	if err := a.store.ping(ctx); err != nil {
		http.Error(w, "database unreachable", http.StatusServiceUnavailable)
		return
	}
	w.Write([]byte("ok\n"))
}

// home sends players to their account, and visitors who aren't logged in
// (most likely new players) to the setup steps.
func (a *app) home(req *request) error {
	if req.account == nil {
		return a.redirect(req, "/connect", "")
	}
	return a.redirect(req, "/account", "")
}

// --- Logging in and out ---

func (a *app) loginForm(req *request) error {
	if req.account != nil {
		return a.redirect(req, "/account", "")
	}
	a.keepSession(req)
	a.render(req, http.StatusOK, "login", view{Title: "Log in"})
	return nil
}

func (a *app) login(req *request) error {
	ip := clientIP(req.r)
	userID := strings.TrimSpace(req.r.PostFormValue("userid"))
	password := req.r.PostFormValue("password")
	form := map[string]string{"userid": userID}
	fail := func(msg string) error {
		a.render(req, http.StatusOK, "login", view{Title: "Log in", Error: msg, Form: form})
		return nil
	}

	// Every attempt counts until it turns out to be right, so parallel
	// requests can't get more guesses than the limit.
	if !a.loginFails.take(ip) {
		log.Printf("login from %s refused: too many failed logins", ip)
		return fail("Too many failed logins from this computer. Wait a few minutes and try again.")
	}
	acc, err := a.store.accountByUserID(req.r.Context(), userID)
	if err != nil {
		a.loginFails.release(ip)
		return err
	}
	if acc == nil || !passwordMatches(acc.passHash, password) {
		return fail("Wrong username or password.")
	}
	a.loginFails.release(ip)
	if acc.Blocked() {
		return fail("This account is blocked. Ask an admin if you think that's a mistake.")
	}
	if acc.Banned() {
		return fail("This account is banned until " + acc.BannedUntil().Format("2006-01-02 15:04 MST") + ".")
	}

	a.sessions.end(req.w, req.sess)
	req.sess = a.sessions.start(req.w, acc.ID, loginTTL)
	return a.redirect(req, "/account", "")
}

func (a *app) logout(req *request) error {
	a.sessions.end(req.w, req.sess)
	http.Redirect(req.w, req.r, "/login", http.StatusSeeOther)
	return nil
}

// --- Registering ---

var userIDPattern = regexp.MustCompile(`^[A-Za-z0-9_]{4,23}$`)

func (a *app) registerForm(req *request) error {
	if req.account != nil {
		return a.redirect(req, "/account", "")
	}
	a.keepSession(req)
	a.render(req, http.StatusOK, "register", view{Title: "Create an account", Form: map[string]string{"sex": "M"}})
	return nil
}

func (a *app) register(req *request) error {
	f := req.r.PostFormValue
	form := map[string]string{
		"userid":    strings.TrimSpace(f("userid")),
		"email":     strings.TrimSpace(f("email")),
		"birthdate": strings.TrimSpace(f("birthdate")),
		"sex":       f("sex"),
	}
	fail := func(msg string) error {
		a.render(req, http.StatusOK, "register", view{Title: "Create an account", Error: msg, Form: form})
		return nil
	}

	ip := clientIP(req.r)
	if !userIDPattern.MatchString(form["userid"]) {
		return fail("Usernames are 4 to 23 letters, digits or _.")
	}
	if msg := checkPassword(f("password"), f("password2")); msg != "" {
		return fail(msg)
	}
	if form["sex"] != "M" && form["sex"] != "F" {
		return fail("Choose male or female.")
	}
	if msg := checkEmail(form["email"]); msg != "" {
		return fail(msg)
	}
	if msg := checkBirthdate(form["birthdate"]); msg != "" {
		return fail(msg)
	}

	// Taken just before creating the account, so a form with a typo in it
	// doesn't use up a sign-up, and given back if no account was made.
	if !a.signups.take(ip) {
		log.Printf("sign-up from %s refused: too many accounts created", ip)
		return fail("Too many accounts created from this computer recently. Try again later.")
	}
	id, err := a.store.createAccount(req.r.Context(), form["userid"], hashPassword(f("password")),
		form["sex"], form["email"], form["birthdate"])
	if err != nil {
		a.signups.release(ip)
	}
	if errors.Is(err, errUserIDTaken) {
		return fail("That username is already taken.")
	}
	if err != nil {
		return err
	}
	log.Printf("registered account %d %q from %s", id, form["userid"], ip)

	a.sessions.end(req.w, req.sess)
	req.sess = a.sessions.start(req.w, id, loginTTL)
	return a.redirect(req, "/account", "Your account is ready. Log in to the game with the same username and password; see How to connect to set up your client.")
}

// --- The player's own account ---

type accountData struct {
	Characters []Character
}

func (a *app) account(req *request) error {
	chars, err := a.store.characters(req.r.Context(), req.account.ID)
	if err != nil {
		return err
	}
	a.render(req, http.StatusOK, "account", view{
		Title: "Your account",
		Form:  map[string]string{"email": req.account.Email, "birthdate": req.account.Birthdate},
		Data:  accountData{Characters: chars},
	})
	return nil
}

func (a *app) accountError(req *request, msg string) error {
	chars, err := a.store.characters(req.r.Context(), req.account.ID)
	if err != nil {
		return err
	}
	a.render(req, http.StatusOK, "account", view{
		Title: "Your account",
		Error: msg,
		Form:  map[string]string{"email": req.r.PostFormValue("email"), "birthdate": req.r.PostFormValue("birthdate")},
		Data:  accountData{Characters: chars},
	})
	return nil
}

func (a *app) changePassword(req *request) error {
	f := req.r.PostFormValue
	// Someone at a PC left logged in could otherwise guess the password
	// here as fast as they like.
	key := strconv.Itoa(req.account.ID)
	if !a.passwordTries.take(key) {
		return a.accountError(req, "Too many wrong passwords. Wait a few minutes and try again.")
	}
	if !passwordMatches(req.account.passHash, f("current")) {
		return a.accountError(req, "Your current password is wrong.")
	}
	a.passwordTries.release(key)
	if msg := checkPassword(f("password"), f("password2")); msg != "" {
		return a.accountError(req, msg)
	}
	if err := a.store.setPassword(req.r.Context(), req.account.ID, hashPassword(f("password"))); err != nil {
		return err
	}
	a.sessions.endAccount(req.account.ID, req.sess)
	return a.redirect(req, "/account", "Password changed. Use the new one next time you log in to the game.")
}

func (a *app) changeDetails(req *request) error {
	email := strings.TrimSpace(req.r.PostFormValue("email"))
	birthdate := strings.TrimSpace(req.r.PostFormValue("birthdate"))
	if msg := checkEmail(email); msg != "" {
		return a.accountError(req, msg)
	}
	if msg := checkBirthdate(birthdate); msg != "" {
		return a.accountError(req, msg)
	}
	if err := a.store.setDetails(req.r.Context(), req.account.ID, email, birthdate); err != nil {
		return err
	}
	return a.redirect(req, "/account", "Details saved.")
}

// --- Checks and helpers ---

// rAthena compares MD5(password) as lowercase hex when use_MD5_passwords is on.
func hashPassword(pw string) string {
	sum := md5.Sum([]byte(pw))
	return hex.EncodeToString(sum[:])
}

func passwordMatches(stored, pw string) bool {
	return subtle.ConstantTimeCompare([]byte(strings.ToLower(stored)), []byte(hashPassword(pw))) == 1
}

func checkPassword(pw, again string) string {
	// The client sends at most 23 characters of password.
	if len(pw) < 6 || len(pw) > 23 {
		return "Passwords are 6 to 23 characters."
	}
	for _, c := range pw {
		if c < 0x21 || c > 0x7e {
			return "Passwords can only use letters, digits and symbols you can type in the game, with no spaces."
		}
	}
	if pw != again {
		return "The two passwords don't match."
	}
	return ""
}

func checkEmail(email string) string {
	addr, err := mail.ParseAddress(email)
	if err != nil || addr.Address != email || len(email) > 39 {
		return "Enter a valid email address of up to 39 characters."
	}
	return ""
}

// The birthdate is what the game client asks for when deleting a
// character (char_del_option: 2), so it has to be right.
func checkBirthdate(d string) string {
	t, err := time.Parse("2006-01-02", d)
	if err != nil || t.Year() < 1900 || t.After(time.Now()) {
		return "Enter your birthdate. The game asks for it when you delete a character."
	}
	return ""
}

func clientIP(r *http.Request) string {
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		return r.RemoteAddr
	}
	return host
}
