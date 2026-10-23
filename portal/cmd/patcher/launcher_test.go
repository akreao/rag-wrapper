package main

import (
	"context"
	"net"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestFetchLiveKeepsOnlyPortalLinks(t *testing.T) {
	srv := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		if r.URL.Path != "/patcher/launcher.json" {
			http.NotFound(w, r)
			return
		}
		base := "http://" + r.Host
		w.Write([]byte(`{"server_name":"Midgard","mode":"Renewal","players":true,"online_count":1,
			"online":[{"name":"Pip","job":"Novice","base_level":7,"job_level":6,"map":"new_1-1"}],
			"news":[{"title":"Hi","text":"Welcome"}],
			"links":{"account":"` + base + `/account","evil":"http://elsewhere.example/x","run":"file:///C:/Windows/calc.exe"}}`))
	}))
	defer srv.Close()
	live, err := fetchLive(context.Background(), srv.URL)
	if err != nil {
		t.Fatal(err)
	}
	if live.ServerName != "Midgard" || live.Mode != "Renewal" || !live.Players || len(live.Online) != 1 || live.Online[0].Map != "new_1-1" {
		t.Errorf("%+v", live)
	}
	if len(live.News) != 1 || live.News[0].Title != "Hi" {
		t.Errorf("news: %+v", live.News)
	}
	if len(live.Links) != 1 || live.Links["account"] != srv.URL+"/account" {
		t.Errorf("links: %v", live.Links)
	}
}

func TestFetchLiveFails(t *testing.T) {
	srv := httptest.NewServer(http.NotFoundHandler())
	defer srv.Close()
	if _, err := fetchLive(context.Background(), srv.URL); err == nil {
		t.Fatal("a portal without launcher.json")
	}
}

func TestGameUp(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	port := ln.Addr().(*net.TCPAddr).Port
	ctx := context.Background()
	if !gameUp(ctx, "127.0.0.1", port) {
		t.Error("listening port")
	}
	ln.Close()
	if gameUp(ctx, "127.0.0.1", port) || gameUp(ctx, "", 6900) || gameUp(ctx, "127.0.0.1", 0) {
		t.Error("closed port or no address")
	}
}

func TestGameState(t *testing.T) {
	ln, err := net.Listen("tcp", "127.0.0.1:0")
	if err != nil {
		t.Fatal(err)
	}
	defer ln.Close()
	up := &liveInfo{Address: "127.0.0.1", LoginPort: ln.Addr().(*net.TCPAddr).Port}
	down := &liveInfo{Address: "127.0.0.1", LoginPort: 0}
	ctx := context.Background()
	for _, c := range []struct {
		name       string
		live, last *liveInfo
		want       int
	}{
		{"portal answers, game up", up, nil, upYes},
		{"portal answers, game down", down, up, upNo},
		{"no portal, last address answers", nil, up, upYes},
		// Not Offline: the launcher never reached the portal.
		{"no portal, last address doesn't answer", nil, down, upNoPortal},
		{"no portal yet", nil, nil, upNoPortal},
	} {
		if got := gameState(ctx, c.live, c.last); got != c.want {
			t.Errorf("%s: %d, want %d", c.name, got, c.want)
		}
	}
}

func TestSettings(t *testing.T) {
	dir := t.TempDir()
	if s := loadSettings(dir); s.Theme != themeSystem || s.KeepOpen || s.Tab != "" {
		t.Errorf("defaults: %+v", s)
	}
	if err := saveSettings(dir, settings{KeepOpen: true, Theme: themeDark, Tab: "news", ServerName: "Midgard"}); err != nil {
		t.Fatal(err)
	}
	if s := loadSettings(dir); !s.KeepOpen || s.Theme != themeDark || s.Tab != "news" || s.ServerName != "Midgard" {
		t.Errorf("saved: %+v", s)
	}
	os.WriteFile(filepath.Join(dir, settingsName), []byte(`{"theme":"purple"}`), 0o644)
	if s := loadSettings(dir); s.Theme != themeSystem {
		t.Errorf("unknown theme: %+v", s)
	}
}

func TestStartRepair(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, defaultGRF), []byte("old"), 0o644)
	os.WriteFile(filepath.Join(dir, "data.grf"), []byte("official"), 0o644)
	saveState(dir, state{Applied: 7})
	if err := startRepair(dir); err != nil {
		t.Fatal(err)
	}
	if _, err := os.Stat(filepath.Join(dir, defaultGRF)); !os.IsNotExist(err) {
		t.Error("rag-wrapper.grf kept")
	}
	if b, _ := os.ReadFile(filepath.Join(dir, "data.grf")); string(b) != "official" {
		t.Error("data.grf touched")
	}
	if st := loadState(dir); st.Applied != 0 {
		t.Errorf("state %+v", st)
	}
	// Every patch applies again.
	list := []plistEntry{{1, "a.thor"}, {2, "b.thor"}}
	if got := pending(list, loadState(dir).Applied); len(got) != 2 {
		t.Errorf("pending %v", got)
	}
	// And again with nothing to remove.
	if err := startRepair(dir); err != nil {
		t.Fatal(err)
	}
}

func TestPatchesCantReplaceLauncherSettings(t *testing.T) {
	if _, err := clientPath(t.TempDir(), strings.ToUpper(settingsName)); err == nil {
		t.Error("a patch may overwrite the launcher's settings")
	}
}
