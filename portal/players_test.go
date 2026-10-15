package main

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

const sampleNews = "# Rules\n\nBe nice.\n\n## Server moved\n\nThe new address is on [How to connect](/connect).\n\n```\n## not a heading\n```\n\n## Welcome\n\nOld post.\n"

func TestLatestPost(t *testing.T) {
	got := string(latestPost([]byte(sampleNews)))
	want := "## Server moved\n\nThe new address is on [How to connect](/connect).\n\n```\n## not a heading\n```\n\n"
	if got != want {
		t.Fatalf("latestPost = %q", got)
	}
	if got := latestPost([]byte("# Rules\n\nNo posts yet.\n")); len(got) != 0 {
		t.Fatalf("no posts: %q", got)
	}
	if got := string(latestPost([]byte("## Only\ntext\n# Next\n"))); got != "## Only\ntext\n" {
		t.Fatalf("stops at a level 1 heading: %q", got)
	}
}

func writeNews(t *testing.T, content string) string {
	t.Helper()
	dir := t.TempDir()
	os.MkdirAll(filepath.Join(dir, "portal"), 0o755)
	os.WriteFile(filepath.Join(dir, "portal", "news.md"), []byte(content), 0o644)
	return dir
}

func TestNewsLeavesOutHTMLAndScriptLinks(t *testing.T) {
	n := connectInfo{Overrides: writeNews(t, "## Hi\n\n<script>alert(1)</script>\n\n[x](javascript:alert(1)) <b>bold</b> **ok** & <\n")}.news()
	body := string(n.Body)
	for _, bad := range []string{"<script", "javascript:", "<b>"} {
		if strings.Contains(body, bad) {
			t.Errorf("news has %q: %s", bad, body)
		}
	}
	if !strings.Contains(body, "<strong>ok</strong>") || !strings.Contains(body, "&amp; &lt;") {
		t.Errorf("news didn't render Markdown or escape text: %s", body)
	}
	if n.Latest != n.Body {
		t.Errorf("one post: latest should be the whole file")
	}
}

func TestNewsMissing(t *testing.T) {
	if n := (connectInfo{Overrides: t.TempDir()}).news(); !n.Missing || n.Body != "" {
		t.Fatal(n)
	}
	if n := (connectInfo{Overrides: writeNews(t, " \n\n")}).news(); !n.Missing {
		t.Fatal("blank file should count as no news")
	}
}

func TestNewsPages(t *testing.T) {
	h, dir := patcherSetup(t, "1 0001_client.thor\n")
	rec := get(h, "/news")
	if rec.Code != 200 || !strings.Contains(rec.Body.String(), "No news yet.") {
		t.Fatalf("no news: %d %s", rec.Code, rec.Body)
	}
	if body := get(h, "/patcher/").Body.String(); strings.Contains(body, `id="news"`) {
		t.Fatal("patcher page shows an empty news box")
	}

	os.MkdirAll(filepath.Join(dir, "portal"), 0o755)
	os.WriteFile(filepath.Join(dir, "portal", "news.md"), []byte(sampleNews), 0o644)
	body := get(h, "/news").Body.String()
	if strings.Contains(body, "<h1>News</h1>") {
		t.Error("a news file with its own title gets a second one")
	}
	for _, want := range []string{"<h1>Rules</h1>", "Be nice.", "<h2>Server moved</h2>", "<h2>Welcome</h2>"} {
		if !strings.Contains(body, want) {
			t.Errorf("news page has no %q", want)
		}
	}
	body = get(h, "/patcher/").Body.String()
	if !strings.Contains(body, "<h2>Server moved</h2>") || strings.Contains(body, "Welcome") || strings.Contains(body, "Be nice.") {
		t.Errorf("patcher page should show only the newest post:\n%s", body)
	}
	if !strings.Contains(body, `href="http://example.com/news" data-open`) {
		t.Errorf("patcher page has no link to the news page:\n%s", body)
	}
}

func TestThousands(t *testing.T) {
	for n, want := range map[int64]string{0: "0", 999: "999", 1000: "1,000", 1234567: "1,234,567", -1000: "-1,000"} {
		if got := thousands(n); got != want {
			t.Errorf("thousands(%d) = %q", n, got)
		}
	}
}

func TestRankingsURL(t *testing.T) {
	d := rankingsData{Class: -1}
	if d.URL(rankByLevel) != "/rankings" || d.URL(rankByZeny) != "/rankings?by=zeny" {
		t.Fatal(d.URL(rankByLevel), d.URL(rankByZeny))
	}
	d.Class = 4008
	if d.URL(rankByLevel) != "/rankings?class=4008" || d.URL(rankByZeny) != "/rankings?by=zeny&class=4008" {
		t.Fatal(d.URL(rankByLevel), d.URL(rankByZeny))
	}
}
