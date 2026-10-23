package main

import (
	"context"
	"encoding/json"
	"log"
	"net/http"
)

// The launcher (cmd/patcher) shows the server at a glance beside patching:
// the news, who's online and the top characters, like the home page. It
// reads them from /patcher/launcher.json, which it fetches again now and
// then while it's open. None of it is secret or signed: patching trusts
// only the signed /patcher/info.json.

// How much the launcher gets.
const (
	launcherNews   = 10
	launcherOnline = 100
	launcherTop    = 10
)

type launcherCharacter struct {
	Name      string `json:"name"`
	Job       string `json:"job"`
	BaseLevel int    `json:"base_level"`
	JobLevel  int    `json:"job_level"`
	Map       string `json:"map,omitempty"`
}

type launcherInfo struct {
	ServerName string `json:"server_name"`
	Mode       string `json:"mode"`
	ClientDate string `json:"client_date"`
	// Where the game's login server is, so the launcher can check it's up.
	Address   string `json:"address"`
	LoginPort int    `json:"login_port"`
	// Players is false when the database couldn't be read; the lists are
	// then empty rather than wrong.
	Players     bool                `json:"players"`
	OnlineCount int                 `json:"online_count"`
	Online      []launcherCharacter `json:"online"`
	Top         []launcherCharacter `json:"top"`
	News        []newsPostText      `json:"news"`
	Links       map[string]string   `json:"links"`
}

func (a *app) launcherInfo(w http.ResponseWriter, r *http.Request) {
	base := a.conn.BaseURL(r)
	d := launcherInfo{
		ServerName: a.conn.ServerName(),
		Mode:       a.conn.Mode(),
		ClientDate: a.conn.ClientDate(),
		Address:    a.conn.Address,
		LoginPort:  a.conn.LoginPort(),
		Online:     []launcherCharacter{},
		Top:        []launcherCharacter{},
		News:       a.conn.newsText(launcherNews),
		Links: map[string]string{
			"home":     base + "/",
			"account":  base + "/account",
			"register": base + "/register",
			"connect":  base + "/connect",
			"news":     base + "/news",
			"online":   base + "/online",
			"rankings": base + "/rankings",
			"market":   base + "/market",
			"items":    base + "/items",
			"monsters": base + "/monsters",
		},
	}
	if d.News == nil {
		d.News = []newsPostText{}
	}
	if a.store != nil {
		ctx, cancel := context.WithTimeout(r.Context(), dbTimeout)
		defer cancel()
		online, err := a.store.onlineCharacters(ctx)
		if err == nil {
			d.OnlineCount = len(online)
			d.Online = launcherCharacters(online, launcherOnline, true)
			var top []PlayerCharacter
			if top, err = a.store.rankings(ctx, rankByLevel, -1, a.adminGroup); err == nil {
				d.Top = launcherCharacters(top, launcherTop, false)
				d.Players = true
			}
		}
		if err != nil {
			log.Printf("launcher: %v", err)
			d.OnlineCount = 0
			d.Online = []launcherCharacter{}
		}
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	json.NewEncoder(w).Encode(d)
}

func launcherCharacters(list []PlayerCharacter, limit int, withMap bool) []launcherCharacter {
	out := []launcherCharacter{}
	for _, c := range list[:min(len(list), limit)] {
		lc := launcherCharacter{Name: c.Name, Job: c.JobName(), BaseLevel: c.BaseLevel, JobLevel: c.JobLevel}
		if withMap {
			lc.Map = c.Map
		}
		out = append(out, lc)
	}
	return out
}
