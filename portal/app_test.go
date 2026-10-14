package main

import (
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

func TestLimiter(t *testing.T) {
	l := newLimiter(2, time.Minute)
	l.add("1.2.3.4")
	if !l.allowed("1.2.3.4") {
		t.Fatal("blocked after one hit")
	}
	l.add("1.2.3.4")
	if l.allowed("1.2.3.4") {
		t.Fatal("allowed after two hits")
	}
	if !l.allowed("5.6.7.8") {
		t.Fatal("other key blocked")
	}
}

func TestJobName(t *testing.T) {
	if jobName(4008) != "Lord Knight" || jobName(0) != "Novice" || jobName(123456) != "Class 123456" {
		t.Fatal(jobName(4008), jobName(0))
	}
}
