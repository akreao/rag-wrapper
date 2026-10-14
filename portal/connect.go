package main

import (
	"bytes"
	"encoding/xml"
	"net"
	"net/http"
	"strings"
	"time"
)

// connectInfo is what players need to point their game client at this
// server. It comes from the same .env the rest of the stack reads.
type connectInfo struct {
	Address   string // PUBLIC_IP: where the client is sent for login, char and map
	PacketVer string // PACKETVER: the client date the server was built for
	Name      string // SERVER_NAME: shown in the client's server list
}

// serverName tidies SERVER_NAME for the client's server list, which has
// room for a short line of text.
func serverName(s string) string {
	s = strings.Join(strings.Fields(s), " ")
	if s == "" {
		return "LAN server"
	}
	if r := []rune(s); len(r) > 40 {
		s = string(r[:40])
	}
	return s
}

// Ports the client connects to. They are fixed in compose.yaml.
const (
	loginPort = 6900
	charPort  = 6121
	mapPort   = 5121
)

// Loopback is true when the server only advertises this machine, so other
// PCs on the LAN would be sent to themselves.
func (c connectInfo) Loopback() bool {
	ip := net.ParseIP(c.Address)
	return ip != nil && ip.IsLoopback()
}

// ClientDate shows PACKETVER as a date, e.g. 20220406 as 2022-04-06.
func (c connectInfo) ClientDate() string {
	t, err := time.Parse("20060102", c.PacketVer)
	if err != nil {
		return c.PacketVer
	}
	return t.Format("2006-01-02")
}

func (c connectInfo) LoginPort() int { return loginPort }
func (c connectInfo) CharPort() int  { return charPort }
func (c connectInfo) MapPort() int   { return mapPort }

// clientInfo builds a clientinfo.xml that sends the client to this server.
// sclientinfo.xml is the same file under the name Sakray clients look for.
// It declares euc-kr like the stock file. Everything in it is ASCII except
// perhaps SERVER_NAME, which is written as UTF-8.
// registerURL is the portal's sign-up page, which some clients open from
// the login screen.
//
// servicetype korea and langtype 1 are what rAthena's wiki template uses.
// The client sends <version> at login, but rAthena no longer checks it
// (only client_hash_check, which is off), so any number works.
func (c connectInfo) clientInfo(registerURL string) []byte {
	type connection struct {
		Display         string   `xml:"display"`
		Desc            string   `xml:"desc"`
		Balloon         string   `xml:"balloon"`
		Address         string   `xml:"address"`
		Port            int      `xml:"port"`
		Version         int      `xml:"version"`
		LangType        int      `xml:"langtype"`
		RegistrationWeb string   `xml:"registrationweb"`
		Loading         []string `xml:"loading>image"`
	}
	type clientinfo struct {
		XMLName     xml.Name   `xml:"clientinfo"`
		Desc        string     `xml:"desc"`
		ServiceType string     `xml:"servicetype"`
		ServerType  string     `xml:"servertype"`
		Connection  connection `xml:"connection"`
	}
	v := clientinfo{
		Desc:        "Ragnarok Client Information",
		ServiceType: "korea",
		ServerType:  "primary",
		Connection: connection{
			Display:         c.Name,
			Desc:            "Ragnarok Online",
			Balloon:         "Ragnarok Online on " + c.Address,
			Address:         c.Address,
			Port:            loginPort,
			Version:         55,
			LangType:        1,
			RegistrationWeb: registerURL,
			Loading:         []string{"loading00.jpg", "loading01.jpg", "loading02.jpg", "loading03.jpg", "loading04.jpg"},
		},
	}
	var buf bytes.Buffer
	buf.WriteString(`<?xml version="1.0" encoding="euc-kr" ?>` + "\n")
	enc := xml.NewEncoder(&buf)
	enc.Indent("", "\t")
	enc.Encode(v) // only fails on types it can't encode, and these are fixed
	buf.WriteString("\n")
	return buf.Bytes()
}

func (a *app) connect(req *request) error {
	a.render(req, http.StatusOK, "connect", view{Title: "How to connect", Data: a.conn})
	return nil
}

// clientInfoFile serves clientinfo.xml or sclientinfo.xml as a download.
func (a *app) clientInfoFile(name string) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		w.Header().Set("Content-Type", "application/xml; charset=utf-8")
		w.Header().Set("Content-Disposition", `attachment; filename="`+name+`"`)
		w.Header().Set("Cache-Control", "no-store")
		w.Write(a.conn.clientInfo("http://" + r.Host + "/register"))
	}
}
