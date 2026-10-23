package main

import (
	"bytes"
	"encoding/xml"
	"fmt"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestClientInfo(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "char_conf.txt"), []byte("// server_name: Old\nserver_name: Prontera_LAN\n"), 0o644)
	c := connectInfo{Address: "192.168.1.20", PacketVer: "20220406", Overrides: dir}
	var got struct {
		ServiceType string `xml:"servicetype"`
		Connection  struct {
			Display         string `xml:"display"`
			Address         string `xml:"address"`
			Port            int    `xml:"port"`
			RegistrationWeb string `xml:"registrationweb"`
			Yellow          []int  `xml:"yellow>admin"`
			AID             []int  `xml:"aid>admin"`
		} `xml:"connection"`
	}
	dec := xml.NewDecoder(bytes.NewReader(c.clientInfo("http://x/register?a=1&b=<2>", []int{2000000, 2000001})))
	// The file declares euc-kr; its content is ASCII, so read it as is.
	dec.CharsetReader = func(_ string, r io.Reader) (io.Reader, error) { return r, nil }
	if err := dec.Decode(&got); err != nil {
		t.Fatal(err)
	}
	if got.ServiceType != "korea" || got.Connection.Display != "Prontera_LAN" || got.Connection.Address != "192.168.1.20" || got.Connection.Port != 6900 ||
		got.Connection.RegistrationWeb != "http://x/register?a=1&b=<2>" {
		t.Fatalf("%+v", got)
	}
	if fmt.Sprint(got.Connection.Yellow, got.Connection.AID) != "[2000000 2000001] [2000000 2000001]" {
		t.Fatalf("yellow %v, aid %v", got.Connection.Yellow, got.Connection.AID)
	}
	if b := string(c.clientInfo("http://x/register", nil)); strings.Contains(b, "<yellow") || strings.Contains(b, "<aid") {
		t.Fatalf("empty GM lists are written:\n%s", b)
	}
	if c.ClientDate() != "2022-04-06" || c.Loopback() {
		t.Fatal(c.ClientDate(), c.Loopback())
	}
	if !(connectInfo{Address: "127.0.0.1"}).Loopback() {
		t.Fatal("127.0.0.1 should be loopback")
	}
	if d := (connectInfo{PacketVer: "20250716"}).ClientDate(); d != "2025-07-16" {
		t.Fatal(d)
	}
}

func TestConnectRoutes(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20250716"})
	if err != nil {
		t.Fatal(err)
	}
	h := a.routes()

	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/", nil))
	if body := rec.Body.String(); rec.Code != http.StatusOK || !strings.Contains(body, "Welcome to rAthena") ||
		!strings.Contains(body, `href="/connect"`) || !strings.Contains(body, "10.0.0.5") ||
		!strings.Contains(body, "The game database isn't answering") {
		t.Fatalf("GET /: %d\n%s", rec.Code, body)
	}

	rec = httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/connect", nil))
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "10.0.0.5") ||
		!strings.Contains(rec.Body.String(), "2025-07-16") || !strings.Contains(rec.Body.String(), "choose <em>Pre-Renewal</em>") {
		t.Fatalf("GET /connect: %d\n%s", rec.Code, rec.Body)
	}

	if rec := get(h, "/connect/sclientinfo.xml"); rec.Code != http.StatusNotFound {
		t.Fatalf("sclientinfo.xml is still served: %d", rec.Code)
	}
	for _, name := range []string{"clientinfo.xml"} {
		rec := httptest.NewRecorder()
		h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "http://10.0.0.5:8080/connect/"+name, nil))
		body := rec.Body.String()
		if rec.Code != http.StatusOK || !strings.Contains(rec.Header().Get("Content-Disposition"), name) ||
			!strings.Contains(body, "<address>10.0.0.5</address>") ||
			!strings.Contains(body, "<registrationweb>http://10.0.0.5:8080/register</registrationweb>") {
			t.Fatalf("GET %s: %d %v\n%s", name, rec.Code, rec.Header(), body)
		}
	}
}

func TestExternalSettings(t *testing.T) {
	got := string(connectInfo{Address: "10.0.0.5"}.externalSettings())
	if !strings.Contains(got, "\r\nAssistAddr = \"10.0.0.5:8888\"\r\n") || strings.Contains(got, "127.0.0.1") ||
		len(got) != len(externalSettingsTemplate)-len("127.0.0.1")+len("10.0.0.5") {
		t.Fatalf("ExternalSettings_kr.lub:\n%s", got)
	}
	// The rest of the translation's file stays, such as the emblem URLs.
	if !strings.Contains(got, "Download = 'http://'..AssistAddr..'/emblem/download'") || !strings.Contains(got, "BaseLevel = 99,") {
		t.Fatalf("ExternalSettings_kr.lub lost the translation's settings:\n%s", got)
	}

	rec := get(newTestRoutes(t), "/connect/ExternalSettings_kr.lub")
	if rec.Code != http.StatusOK || !strings.Contains(rec.Header().Get("Content-Disposition"), "ExternalSettings_kr.lub") ||
		!strings.Contains(rec.Body.String(), `AssistAddr = "10.0.0.5:8888"`) {
		t.Fatalf("GET ExternalSettings_kr.lub: %d %v", rec.Code, rec.Header())
	}
}

func TestLinuxLauncher(t *testing.T) {
	h := newTestRoutes(t)
	rec := get(h, "/connect/play-on-linux.sh")
	if rec.Code != http.StatusOK || !strings.Contains(rec.Header().Get("Content-Disposition"), "play-on-linux.sh") ||
		!strings.HasPrefix(rec.Body.String(), "#!/usr/bin/env bash\n") || !strings.Contains(rec.Body.String(), "patcher="+patcherName+".exe\n") ||
		!strings.Contains(rec.Body.String(), "client="+clientExe+"\n") {
		t.Fatalf("GET play-on-linux.sh: %d %v\n%s", rec.Code, rec.Header(), rec.Body)
	}

	// Without a published patch, players start their own patched client.
	body := get(h, "/connect").Body.String()
	if !strings.Contains(body, "Playing on Linux") || !strings.Contains(body, `href="/connect/play-on-linux.sh"`) ||
		!strings.Contains(body, "run program.exe") || strings.Contains(body, "run WARPGATE.exe") {
		t.Fatalf("connect without a patch:\n%s", body)
	}
}

func newTestRoutes(t *testing.T) http.Handler {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20250716"})
	if err != nil {
		t.Fatal(err)
	}
	return a.routes()
}

func TestServerNameDefault(t *testing.T) {
	if got := (connectInfo{Overrides: t.TempDir()}).ServerName(); got != "rAthena" {
		t.Fatal(got)
	}
}

func TestConnectRenewal(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20250716", Renewal: true})
	if err != nil {
		t.Fatal(err)
	}
	if body := get(a.routes(), "/connect").Body.String(); !strings.Contains(body, "<dd>Renewal</dd>") ||
		!strings.Contains(body, "choose <em>Renewal</em>") || strings.Contains(body, "choose <em>Pre-Renewal</em>") {
		t.Fatalf("connect on a Renewal server:\n%s", body)
	}
}
