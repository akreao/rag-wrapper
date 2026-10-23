package main

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
)

func TestLauncherInfo(t *testing.T) {
	h, dir := patcherSetup(t, "")
	os.MkdirAll(filepath.Join(dir, "portal"), 0o755)
	os.WriteFile(filepath.Join(dir, "portal", "news.md"), []byte("# Rules\n\nBe nice.\n\n## Newest\n\nFirst *post*.\n\n```\n## not a heading\n```\n\n## Older\n\nSecond.\n"), 0o644)
	rec := get(h, "/patcher/launcher.json")
	if rec.Code != 200 || rec.Header().Get("Cache-Control") != "no-store" {
		t.Fatal(rec.Code, rec.Header())
	}
	var d launcherInfo
	if err := json.Unmarshal(rec.Body.Bytes(), &d); err != nil {
		t.Fatal(err)
	}
	if d.Mode != "Pre-Renewal" || d.ClientDate != "2025-07-16" || d.Address != "10.0.0.5" || d.LoginPort != 6900 {
		t.Errorf("server: %+v", d)
	}
	// No database: no players, but empty lists rather than nulls.
	if d.Players || d.Online == nil || d.Top == nil {
		t.Errorf("players: %+v", d)
	}
	if len(d.News) != 2 || d.News[0].Title != "Newest" || d.News[0].Text != "First post.\n\n    ## not a heading" || d.News[1].Title != "Older" {
		t.Errorf("news: %+v", d.News)
	}
	if d.Links["account"] != "http://example.com/account" {
		t.Errorf("links: %v", d.Links)
	}
}

func TestNewsPosts(t *testing.T) {
	src := []byte("## A\n\na\n\n# Other\n\nx\n\n## B\n\nb\n\n## C\n")
	posts := newsPosts(src, 2)
	if len(posts) != 2 || string(posts[0]) != "## A\n\na\n\n" || string(posts[1]) != "## B\n\nb\n\n" {
		t.Errorf("%q", posts)
	}
	if len(newsPosts(src, 10)) != 3 || newsPosts(nil, 5) != nil {
		t.Error("all posts")
	}
}
