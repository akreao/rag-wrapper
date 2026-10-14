package main

import (
	"bytes"
	"encoding/xml"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestClientInfo(t *testing.T) {
	c := connectInfo{Address: "192.168.1.20", PacketVer: "20220406"}
	var got struct {
		ServiceType string `xml:"servicetype"`
		Connection  struct {
			Address         string `xml:"address"`
			Port            int    `xml:"port"`
			RegistrationWeb string `xml:"registrationweb"`
		} `xml:"connection"`
	}
	dec := xml.NewDecoder(bytes.NewReader(c.clientInfo("http://x/register?a=1&b=<2>")))
	// The file declares euc-kr; its content is ASCII, so read it as is.
	dec.CharsetReader = func(_ string, r io.Reader) (io.Reader, error) { return r, nil }
	if err := dec.Decode(&got); err != nil {
		t.Fatal(err)
	}
	if got.ServiceType != "korea" || got.Connection.Address != "192.168.1.20" || got.Connection.Port != 6900 ||
		got.Connection.RegistrationWeb != "http://x/register?a=1&b=<2>" {
		t.Fatalf("%+v", got)
	}
	if c.ClientDate() != "2022-04-06" || c.Loopback() {
		t.Fatal(c.ClientDate(), c.Loopback())
	}
	if !(connectInfo{Address: "127.0.0.1"}).Loopback() {
		t.Fatal("127.0.0.1 should be loopback")
	}
}

func TestConnectRoutes(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20220406"})
	if err != nil {
		t.Fatal(err)
	}
	h := a.routes()

	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, "/connect", nil))
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "10.0.0.5") ||
		!strings.Contains(rec.Body.String(), "2022-04-06") {
		t.Fatalf("GET /connect: %d\n%s", rec.Code, rec.Body)
	}

	for _, name := range []string{"clientinfo.xml", "sclientinfo.xml"} {
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
