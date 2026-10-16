package main

import (
	"crypto/rand"
	"encoding/base64"
	"net/http"
	"sync"
	"time"
)

const (
	sessionCookie = "portal_session"
	anonTTL       = time.Hour
	loginTTL      = 12 * time.Hour
	// maxSessions caps the sessions kept in memory. Past it, starting a
	// session drops a visitor's (not a logged-in player's) to make room, so
	// a request flood can't run the portal out of memory.
	maxSessions = 10000
)

// session is held in memory only: restarting the portal logs everyone out,
// which is fine for a LAN server.
type session struct {
	token     string
	accountID int // 0 until logged in
	csrf      string
	flash     string
	expires   time.Time
}

type sessions struct {
	mu   sync.Mutex
	byID map[string]*session
}

func newSessions() *sessions {
	s := &sessions{byID: map[string]*session{}}
	go func() {
		for range time.Tick(10 * time.Minute) {
			s.prune()
		}
	}()
	return s
}

func (s *sessions) prune() {
	s.mu.Lock()
	defer s.mu.Unlock()
	now := time.Now()
	for k, v := range s.byID {
		if now.After(v.expires) {
			delete(s.byID, k)
		}
	}
}

// get returns the request's session. A visitor without one gets a blank
// session that isn't kept and has no CSRF token; only pages with a form
// keep one for them (see app.keepSession), so browsing the public pages
// stores nothing.
func (s *sessions) get(r *http.Request) *session {
	if c, err := r.Cookie(sessionCookie); err == nil {
		s.mu.Lock()
		sess, ok := s.byID[c.Value]
		if ok && time.Now().After(sess.expires) {
			delete(s.byID, c.Value)
			ok = false
		}
		s.mu.Unlock()
		if ok {
			return sess
		}
	}
	return &session{}
}

// kept reports whether sess is stored, rather than a visitor's blank one.
func (sess *session) kept() bool { return sess.token != "" }

// start issues a fresh session. Logging in always gets a new token, so a
// token seen before login is useless afterwards.
func (s *sessions) start(w http.ResponseWriter, accountID int, ttl time.Duration) *session {
	sess := &session{token: randomToken(), accountID: accountID, csrf: randomToken(), expires: time.Now().Add(ttl)}
	s.mu.Lock()
	if len(s.byID) >= maxSessions {
		s.dropVisitor()
	}
	s.byID[sess.token] = sess
	s.mu.Unlock()
	http.SetCookie(w, &http.Cookie{
		Name:     sessionCookie,
		Value:    sess.token,
		Path:     "/",
		Expires:  sess.expires,
		HttpOnly: true,
		SameSite: http.SameSiteLaxMode,
	})
	return sess
}

// dropVisitor removes one session that isn't logged in, if there is one.
// Map order is random, so it's an arbitrary one. Called with s.mu held.
func (s *sessions) dropVisitor() {
	for k, v := range s.byID {
		if v.accountID == 0 {
			delete(s.byID, k)
			return
		}
	}
}

func (s *sessions) end(w http.ResponseWriter, sess *session) {
	s.mu.Lock()
	delete(s.byID, sess.token)
	s.mu.Unlock()
	http.SetCookie(w, &http.Cookie{Name: sessionCookie, Value: "", Path: "/", MaxAge: -1, HttpOnly: true})
}

// endAccount logs an account out everywhere, after a password change.
func (s *sessions) endAccount(accountID int, keep *session) {
	s.mu.Lock()
	defer s.mu.Unlock()
	for k, v := range s.byID {
		if v.accountID == accountID && v != keep {
			delete(s.byID, k)
		}
	}
}

func (s *sessions) setFlash(sess *session, msg string) {
	s.mu.Lock()
	sess.flash = msg
	s.mu.Unlock()
}

func (s *sessions) takeFlash(sess *session) string {
	s.mu.Lock()
	defer s.mu.Unlock()
	msg := sess.flash
	sess.flash = ""
	return msg
}

func randomToken() string {
	b := make([]byte, 32)
	if _, err := rand.Read(b); err != nil {
		panic(err)
	}
	return base64.RawURLEncoding.EncodeToString(b)
}

// limiter counts events per key (an IP address or an account) in a
// sliding window.
type limiter struct {
	mu     sync.Mutex
	max    int
	window time.Duration
	hits   map[string][]time.Time
}

func newLimiter(max int, window time.Duration) *limiter {
	return &limiter{max: max, window: window, hits: map[string][]time.Time{}}
}

func (l *limiter) recent(key string) []time.Time {
	cutoff := time.Now().Add(-l.window)
	h := l.hits[key]
	i := 0
	for i < len(h) && h[i].Before(cutoff) {
		i++
	}
	h = h[i:]
	if len(h) == 0 {
		delete(l.hits, key)
	} else {
		l.hits[key] = h
	}
	return h
}

// take records an attempt for key and reports whether it was under the
// limit. Checking and recording happen under one lock, so requests sent
// in parallel can't all get in before any of them is counted. An attempt
// over the limit isn't recorded.
func (l *limiter) take(key string) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	h := l.recent(key)
	if len(h) >= l.max {
		return false
	}
	l.hits[key] = append(h, time.Now())
	return true
}

// release gives back an attempt take recorded, for one that turned out
// not to count (a correct password, a sign-up that didn't happen).
func (l *limiter) release(key string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	if h := l.recent(key); len(h) > 0 {
		h = h[:len(h)-1]
		if len(h) == 0 {
			delete(l.hits, key)
		} else {
			l.hits[key] = h
		}
	}
}

// prune forgets keys with nothing left in the window, such as IPs that
// never came back.
func (l *limiter) prune() {
	l.mu.Lock()
	defer l.mu.Unlock()
	for k := range l.hits {
		l.recent(k)
	}
}
