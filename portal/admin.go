package main

import (
	"errors"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"
)

const adminPageSize = 50

type adminListData struct {
	Query    string
	Accounts []*Account
	Page     int
	PrevURL  string
	NextURL  string
}

func (a *app) adminList(req *request) error {
	q := strings.TrimSpace(req.r.URL.Query().Get("q"))
	page, _ := strconv.Atoi(req.r.URL.Query().Get("page"))
	if page < 1 {
		page = 1
	}
	// One extra row tells us whether there's a next page.
	accounts, err := a.store.searchAccounts(req.r.Context(), q, adminPageSize+1, (page-1)*adminPageSize)
	if err != nil {
		return err
	}
	d := adminListData{Query: q, Page: page}
	pageURL := func(p int) string {
		v := url.Values{"page": {strconv.Itoa(p)}}
		if q != "" {
			v.Set("q", q)
		}
		return "/admin?" + v.Encode()
	}
	if len(accounts) > adminPageSize {
		accounts = accounts[:adminPageSize]
		d.NextURL = pageURL(page + 1)
	}
	if page > 1 {
		d.PrevURL = pageURL(page - 1)
	}
	d.Accounts = accounts
	a.render(req, http.StatusOK, "admin", view{Title: "Accounts", Data: d})
	return nil
}

type adminAccountData struct {
	Target     *Account
	Self       bool
	Characters []Character
}

// target loads the account named in the URL, or writes a 404.
func (a *app) target(req *request) (*Account, error) {
	id, err := strconv.Atoi(req.r.PathValue("id"))
	if err != nil {
		a.render(req, http.StatusNotFound, "error", view{Title: "No such account", Error: "There's no account with that number."})
		return nil, nil
	}
	acc, err := a.store.accountByID(req.r.Context(), id)
	if err == nil && acc == nil {
		a.render(req, http.StatusNotFound, "error", view{Title: "No such account", Error: "There's no account with that number."})
	}
	return acc, err
}

func (a *app) adminAccount(req *request) error {
	t, err := a.target(req)
	if t == nil || err != nil {
		return err
	}
	return a.showAdminAccount(req, t, "")
}

func (a *app) showAdminAccount(req *request, t *Account, errMsg string) error {
	chars, err := a.store.characters(req.r.Context(), t.ID)
	if err != nil {
		return err
	}
	a.render(req, http.StatusOK, "admin_account", view{
		Title: t.UserID,
		Error: errMsg,
		Data:  adminAccountData{Target: t, Self: t.ID == req.account.ID, Characters: chars},
	})
	return nil
}

func (a *app) adminSetGroup(req *request) error {
	t, err := a.target(req)
	if t == nil || err != nil {
		return err
	}
	if t.ID == req.account.ID {
		return a.showAdminAccount(req, t, "You can't change your own group; ask another admin.")
	}
	group, err := strconv.Atoi(req.r.PostFormValue("group"))
	known := false
	for _, g := range groups {
		known = known || (err == nil && g.ID == group)
	}
	if !known {
		return a.showAdminAccount(req, t, "Pick a group from the list.")
	}
	if err := a.store.setGroup(req.r.Context(), t.ID, group); err != nil {
		return err
	}
	return a.redirect(req, adminAccountURL(t), t.UserID+" is now in "+groupName(group)+". It applies from their next login.")
}

// banLengths are the choices on the ban form; 0 means blocked until lifted.
var banLengths = map[string]time.Duration{
	"1h":        time.Hour,
	"1d":        24 * time.Hour,
	"7d":        7 * 24 * time.Hour,
	"30d":       30 * 24 * time.Hour,
	"permanent": 0,
}

func (a *app) adminBan(req *request) error {
	t, err := a.target(req)
	if t == nil || err != nil {
		return err
	}
	if t.ID == req.account.ID {
		return a.showAdminAccount(req, t, "You can't ban yourself.")
	}
	length, ok := banLengths[req.r.PostFormValue("length")]
	if !ok {
		return a.showAdminAccount(req, t, "Pick how long the ban lasts.")
	}
	if length == 0 {
		err = a.store.block(req.r.Context(), t.ID)
	} else {
		err = a.store.ban(req.r.Context(), t.ID, time.Now().Add(length))
	}
	if err != nil {
		return err
	}
	a.sessions.endAccount(t.ID, nil)
	return a.redirect(req, adminAccountURL(t), t.UserID+" is banned.")
}

func (a *app) adminUnban(req *request) error {
	t, err := a.target(req)
	if t == nil || err != nil {
		return err
	}
	if err := a.store.unban(req.r.Context(), t.ID); err != nil {
		return err
	}
	return a.redirect(req, adminAccountURL(t), t.UserID+" can log in again.")
}

// adminSetPassword is how a player who forgot their password gets back in:
// there's no email on a LAN server, so an admin sets a new one.
func (a *app) adminSetPassword(req *request) error {
	t, err := a.target(req)
	if t == nil || err != nil {
		return err
	}
	if t.ID == req.account.ID {
		return a.showAdminAccount(req, t, "Change your own password on your account page.")
	}
	pw := req.r.PostFormValue("password")
	if msg := checkPassword(pw, req.r.PostFormValue("password2")); msg != "" {
		return a.showAdminAccount(req, t, msg)
	}
	if err := a.store.setPassword(req.r.Context(), t.ID, hashPassword(pw)); err != nil {
		return err
	}
	a.sessions.endAccount(t.ID, nil)
	return a.redirect(req, adminAccountURL(t), "New password set for "+t.UserID+". Ask them to change it on their account page.")
}

func adminAccountURL(t *Account) string { return "/admin/accounts/" + strconv.Itoa(t.ID) }

// --- Characters ---

type adminCharacterData struct {
	Character *Character
	Owner     *Account
	Items     []Item
}

// character loads the character named in the URL, or writes a 404.
func (a *app) character(req *request) (*Character, error) {
	id, err := strconv.Atoi(req.r.PathValue("id"))
	if err != nil {
		a.render(req, http.StatusNotFound, "error", view{Title: "No such character", Error: "There's no character with that number."})
		return nil, nil
	}
	c, err := a.store.characterByID(req.r.Context(), id)
	if err == nil && c == nil {
		a.render(req, http.StatusNotFound, "error", view{Title: "No such character", Error: "There's no character with that number."})
	}
	return c, err
}

func (a *app) adminCharacter(req *request) error {
	c, err := a.character(req)
	if c == nil || err != nil {
		return err
	}
	return a.showAdminCharacter(req, c, "")
}

func (a *app) showAdminCharacter(req *request, c *Character, errMsg string) error {
	owner, err := a.store.accountByID(req.r.Context(), c.AccountID)
	if err != nil {
		return err
	}
	items, err := a.store.inventory(req.r.Context(), c.ID)
	if err != nil {
		return err
	}
	a.render(req, http.StatusOK, "admin_character", view{
		Title: c.Name,
		Error: errMsg,
		Data:  adminCharacterData{Character: c, Owner: owner, Items: items},
	})
	return nil
}

// adminMoveToSavePoint is for a character stuck somewhere it can't leave,
// such as a map that crashes the client or a spot inside a wall.
func (a *app) adminMoveToSavePoint(req *request) error {
	c, err := a.character(req)
	if c == nil || err != nil {
		return err
	}
	err = a.store.moveToSavePoint(req.r.Context(), c.ID)
	if errors.Is(err, errCharacterOnline) {
		c.Online = true
		return a.showAdminCharacter(req, c, c.Name+" is in game. Ask the player to log out, or use @jumpto or @recall in game.")
	}
	if err != nil {
		return err
	}
	return a.redirect(req, adminCharacterURL(c), c.Name+" will log in at "+c.SaveMap+" ("+strconv.Itoa(c.SaveX)+", "+strconv.Itoa(c.SaveY)+").")
}

func adminCharacterURL(c *Character) string { return "/admin/characters/" + strconv.Itoa(c.ID) }
