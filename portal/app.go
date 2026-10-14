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
	loginFails *limiter
	signups    *limiter
}

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

func newApp(st *store, adminGroup int) (*app, error) {
	funcs := template.FuncMap{
		"groupName": groupName,
		"groups":    func() any { return groups },
		"date":      func(t time.Time) string { return t.Format("2006-01-02 15:04 MST") },
	}
	pages := map[string]*template.Template{}
	for _, name := range []string{"login", "register", "account", "admin", "admin_account", "error"} {
		t, err := template.New("").Funcs(funcs).ParseFS(assets, "templates/layout.html", "templates/"+name+".html")
		if err != nil {
			return nil, err
		}
		pages[name] = t
	}
	return &app{
		store:      st,
		sessions:   newSessions(),
		pages:      pages,
		adminGroup: adminGroup,
		loginFails: newLimiter(10, 15*time.Minute),
		signups:    newLimiter(5, time.Hour),
	}, nil
}

func (a *app) routes() http.Handler {
	static, _ := fs.Sub(assets, "static")
	mux := http.NewServeMux()
	mux.Handle("GET /static/", http.StripPrefix("/static/", http.FileServerFS(static)))
	mux.HandleFunc("GET /healthz", a.healthz)
	mux.HandleFunc("GET /{$}", a.home)
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
		req := &request{w: w, r: r, sess: a.sessions.get(w, r)}
		if r.Method == http.MethodPost {
			r.Body = http.MaxBytesReader(w, r.Body, 64<<10)
			if err := r.ParseForm(); err != nil {
				http.Error(w, "bad form", http.StatusBadRequest)
				return
			}
			if subtle.ConstantTimeCompare([]byte(r.PostFormValue("csrf")), []byte(req.sess.csrf)) != 1 {
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

// redirect sends the browser on after a successful POST, with a message
// for the next page.
func (a *app) redirect(req *request, to, flash string) error {
	if flash != "" {
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

func (a *app) home(w http.ResponseWriter, r *http.Request) {
	http.Redirect(w, r, "/account", http.StatusSeeOther)
}

// --- Logging in and out ---

func (a *app) loginForm(req *request) error {
	if req.account != nil {
		return a.redirect(req, "/account", "")
	}
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

	if !a.loginFails.allowed(ip) {
		return fail("Too many failed logins from this computer. Wait a few minutes and try again.")
	}
	acc, err := a.store.accountByUserID(req.r.Context(), userID)
	if err != nil {
		return err
	}
	if acc == nil || !passwordMatches(acc.passHash, password) {
		a.loginFails.add(ip)
		return fail("Wrong username or password.")
	}
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
	if !a.signups.allowed(ip) {
		return fail("Too many accounts created from this computer recently. Try again later.")
	}
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

	id, err := a.store.createAccount(req.r.Context(), form["userid"], hashPassword(f("password")),
		form["sex"], form["email"], form["birthdate"])
	if errors.Is(err, errUserIDTaken) {
		return fail("That username is already taken.")
	}
	if err != nil {
		return err
	}
	a.signups.add(ip)
	log.Printf("registered account %d %q from %s", id, form["userid"], ip)

	a.sessions.end(req.w, req.sess)
	req.sess = a.sessions.start(req.w, id, loginTTL)
	return a.redirect(req, "/account", "Your account is ready. Log in to the game with the same username and password.")
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
	if !passwordMatches(req.account.passHash, f("current")) {
		return a.accountError(req, "Your current password is wrong.")
	}
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
