package main

import (
	"net/http"
	"net/http/httptest"
	"net/url"
	"strings"
	"testing"
)

func testApp(t *testing.T) *app {
	t.Helper()
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20250716", Overrides: t.TempDir()})
	if err != nil {
		t.Fatal(err)
	}
	return a
}

func (s *sessions) count() int {
	s.mu.Lock()
	defer s.mu.Unlock()
	return len(s.byID)
}

// Browsing the public pages stores nothing; only a page with a form does.
func TestVisitorsGetSessionsOnlyForForms(t *testing.T) {
	a := testApp(t)
	h := a.routes()
	for range 100 {
		if rec := get(h, "/connect"); rec.Code != http.StatusOK || rec.Header().Get("Set-Cookie") != "" {
			t.Fatalf("GET /connect: %d, cookie %q", rec.Code, rec.Header().Get("Set-Cookie"))
		}
	}
	if n := a.sessions.count(); n != 0 {
		t.Fatalf("%d sessions after browsing", n)
	}
	rec := get(h, "/login")
	if !strings.HasPrefix(rec.Header().Get("Set-Cookie"), sessionCookie+"=") || a.sessions.count() != 1 {
		t.Fatalf("login form: cookie %q, %d sessions", rec.Header().Get("Set-Cookie"), a.sessions.count())
	}
}

// A POST without a session is an expired form, even with an empty token.
func TestPostWithoutSessionIsRefused(t *testing.T) {
	h := testApp(t).routes()
	req := httptest.NewRequest(http.MethodPost, "/login", strings.NewReader(url.Values{"csrf": {""}, "userid": {"admin"}}.Encode()))
	req.Header.Set("Content-Type", "application/x-www-form-urlencoded")
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, req)
	if rec.Code != http.StatusForbidden {
		t.Fatal(rec.Code)
	}
}

// Past the cap, new sessions push out visitors' but never players'.
func TestSessionCap(t *testing.T) {
	s := newSessions()
	w := httptest.NewRecorder()
	player := s.start(w, 42, loginTTL)
	for range maxSessions + 50 {
		s.start(w, 0, anonTTL)
	}
	if n := s.count(); n != maxSessions {
		t.Fatalf("%d sessions, want %d", n, maxSessions)
	}
	s.mu.Lock()
	_, ok := s.byID[player.token]
	s.mu.Unlock()
	if !ok {
		t.Fatal("a logged-in session was dropped")
	}
}
