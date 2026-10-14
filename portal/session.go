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

// get returns the request's session, starting an anonymous one if needed.
func (s *sessions) get(w http.ResponseWriter, r *http.Request) *session {
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
	return s.start(w, 0, anonTTL)
}

// start issues a fresh session. Logging in always gets a new token, so a
// token seen before login is useless afterwards.
func (s *sessions) start(w http.ResponseWriter, accountID int, ttl time.Duration) *session {
	sess := &session{token: randomToken(), accountID: accountID, csrf: randomToken(), expires: time.Now().Add(ttl)}
	s.mu.Lock()
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

// limiter counts events per key (an IP address) in a sliding window.
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

// allowed reports whether key is still under the limit.
func (l *limiter) allowed(key string) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	return len(l.recent(key)) < l.max
}

func (l *limiter) add(key string) {
	l.mu.Lock()
	defer l.mu.Unlock()
	l.hits[key] = append(l.recent(key), time.Now())
}
