package main

import (
	"net/http"
	"net/http/httptest"
	"sync"
	"sync/atomic"
	"testing"
	"time"
)

func TestHashPasswordMatchesMySQLMD5(t *testing.T) {
	// SELECT MD5('changeme123')
	if got := hashPassword("changeme123"); got != "c9cdcb06301f9c79e2d20c2fdeda0a02" {
		t.Fatalf("hashPassword = %q", got)
	}
	if !passwordMatches("5F4DCC3B5AA765D61D8327DEB882CF99", "password") {
		t.Fatal("stored hashes should compare case-insensitively")
	}
	if passwordMatches(hashPassword("a"), "b") {
		t.Fatal("different passwords matched")
	}
}

func TestChecks(t *testing.T) {
	for _, c := range []struct {
		pw, again string
		ok        bool
	}{
		{"secret1", "secret1", true},
		{"short", "short", false},
		{"has space", "has space", false},
		{"secret1", "secret2", false},
		{"123456789012345678901234", "123456789012345678901234", false},
	} {
		if got := checkPassword(c.pw, c.again) == ""; got != c.ok {
			t.Errorf("checkPassword(%q, %q) ok = %v", c.pw, c.again, got)
		}
	}
	for email, ok := range map[string]bool{
		"a@b.com":       true,
		"Bob <a@b.com>": false,
		"not-an-email":  false,
		"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa@b.com": false,
	} {
		if got := checkEmail(email) == ""; got != ok {
			t.Errorf("checkEmail(%q) ok = %v", email, got)
		}
	}
	for d, ok := range map[string]bool{
		"1990-05-17": true,
		"":           false,
		"17/05/1990": false,
		time.Now().AddDate(1, 0, 0).Format("2006-01-02"): false,
	} {
		if got := checkBirthdate(d) == ""; got != ok {
			t.Errorf("checkBirthdate(%q) ok = %v", d, got)
		}
	}
}

func TestDeleteCode(t *testing.T) {
	for birthdate, want := range map[string]string{
		"1995-12-31": "951231",
		"2004-02-09": "040209",
		"":           "",
	} {
		if got := deleteCode(birthdate); got != want {
			t.Errorf("deleteCode(%q) = %q, want %q", birthdate, got, want)
		}
	}
}

func TestLimiter(t *testing.T) {
	l := newLimiter(2, time.Minute)
	if !l.take("1.2.3.4") || !l.take("1.2.3.4") {
		t.Fatal("blocked under the limit")
	}
	if l.take("1.2.3.4") {
		t.Fatal("allowed over the limit")
	}
	if !l.take("5.6.7.8") {
		t.Fatal("other key blocked")
	}
	l.release("1.2.3.4")
	if !l.take("1.2.3.4") {
		t.Fatal("release didn't give the attempt back")
	}
}

// Parallel attempts mustn't all get in before any is counted.
func TestLimiterParallel(t *testing.T) {
	l := newLimiter(10, time.Minute)
	var got atomic.Int32
	var wg sync.WaitGroup
	for range 200 {
		wg.Add(1)
		go func() {
			defer wg.Done()
			if l.take("1.2.3.4") {
				got.Add(1)
			}
		}()
	}
	wg.Wait()
	if got.Load() != 10 {
		t.Fatalf("%d attempts got in, want 10", got.Load())
	}
}

func TestLimiterPrune(t *testing.T) {
	l := newLimiter(1, time.Millisecond)
	l.take("1.2.3.4")
	time.Sleep(5 * time.Millisecond)
	l.prune()
	if len(l.hits) != 0 {
		t.Fatal(l.hits)
	}
}

func TestJobName(t *testing.T) {
	if jobName(4008) != "Lord Knight" || jobName(0) != "Novice" || jobName(123456) != "Class 123456" {
		t.Fatal(jobName(4008), jobName(0))
	}
}

func TestItemLabelAndCards(t *testing.T) {
	it := Item{Name: "Katana", Refine: 7, cards: [4]int{4001, 0, 4002, 0}}
	if it.Label() != "+7 Katana" {
		t.Fatal(it.Label())
	}
	if ids := it.cardIDs(); len(ids) != 2 || ids[0] != 4001 || ids[1] != 4002 {
		t.Fatal(ids)
	}
	for _, c0 := range []int{card0Forge, card0Create, card0Pet} {
		if ids := (Item{cards: [4]int{c0, 5, 6, 7}}).cardIDs(); ids != nil {
			t.Errorf("card0 %#x: %v", c0, ids)
		}
	}
}

func TestAtSavePoint(t *testing.T) {
	c := Character{LastMap: "prontera", LastX: 1, LastY: 2, SaveMap: "prontera", SaveX: 1, SaveY: 2}
	if !c.AtSavePoint() {
		t.Fatal("same spot")
	}
	c.LastX = 3
	if c.AtSavePoint() {
		t.Fatal("different spot")
	}
}

func TestSection(t *testing.T) {
	for path, want := range map[string]string{
		"/": "home", "/connect": "connect", "/items/501": "database", "/monsters": "database",
		"/admin/accounts/3": "admin", "/news": "news",
	} {
		if got := section(path); got != want {
			t.Errorf("section(%q) = %q, want %q", path, got, want)
		}
	}
}

func TestStaticFilesCaching(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20220406"})
	if err != nil {
		t.Fatal(err)
	}
	h := a.routes()
	for _, name := range []string{"style.css", "theme.js", "poring.svg", "fonts/nunito.woff2"} {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/static/"+name, nil))
		etag := rec.Header().Get("ETag")
		if rec.Code != http.StatusOK || etag == "" || rec.Header().Get("Cache-Control") != "no-cache" {
			t.Fatalf("%s: code %d, ETag %q, Cache-Control %q", name, rec.Code, etag, rec.Header().Get("Cache-Control"))
		}
		req := httptest.NewRequest(http.MethodGet, "/static/"+name, nil)
		req.Header.Set("If-None-Match", etag)
		rec = httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusNotModified || rec.Body.Len() != 0 {
			t.Errorf("%s with matching If-None-Match: code %d, %d bytes", name, rec.Code, rec.Body.Len())
		}
		req = httptest.NewRequest(http.MethodGet, "/static/"+name, nil)
		req.Header.Set("If-None-Match", `"stale"`)
		rec = httptest.NewRecorder()
		h.ServeHTTP(rec, req)
		if rec.Code != http.StatusOK || rec.Body.Len() == 0 {
			t.Errorf("%s with stale If-None-Match: code %d, %d bytes", name, rec.Code, rec.Body.Len())
		}
	}
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/static/missing.css", nil))
	if rec.Code != http.StatusNotFound || rec.Header().Get("ETag") != "" {
		t.Errorf("missing file: code %d, ETag %q", rec.Code, rec.Header().Get("ETag"))
	}
}
