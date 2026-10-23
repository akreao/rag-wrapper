package main

import (
	"html/template"
	"log"
	"net/http"
)

// The home page: the server at a glance, for players and visitors alike.
// It shows what's there and leaves out what the database can't give, so a
// database hiccup doesn't take the front page down.

// homeListLength is how many characters the home page's lists show.
const homeListLength = 5

type homeData struct {
	Conn        connectInfo
	News        template.HTML // the newest post, if there's a news file
	Online      []PlayerCharacter
	OnlineCount int
	Top         []PlayerCharacter
	NoDatabase  bool // the online and ranking lists couldn't be read
}

func (a *app) home(req *request) error {
	d := homeData{Conn: a.conn, News: a.conn.news().Latest}
	if a.store == nil {
		d.NoDatabase = true
	} else {
		ctx := req.r.Context()
		online, err := a.store.onlineCharacters(ctx)
		if err == nil {
			d.OnlineCount = len(online)
			d.Online = online[:min(len(online), homeListLength)]
			var top []PlayerCharacter
			if top, err = a.store.rankings(ctx, rankByLevel, -1, a.adminGroup); err == nil {
				d.Top = top[:min(len(top), homeListLength)]
			}
		}
		if err != nil {
			log.Printf("home: %v", err)
			d.NoDatabase = true
		}
	}
	a.render(req, http.StatusOK, "home", view{Title: "Home", Data: d})
	return nil
}
