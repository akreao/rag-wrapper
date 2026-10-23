package main

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net"
	"os"
	"path/filepath"
	"strconv"
	"time"
)

// What the launcher shows beside patching (the Play, News and Players
// tabs), and the player's own launcher settings. Nothing here changes the
// client: patching (patch.go) only trusts the signed info.json.

// settingsName is the launcher's own settings file, beside it in the
// client folder.
const settingsName = "rag-wrapper-launcher.json"

// liveInfo is the portal's /patcher/launcher.json.
type liveInfo struct {
	ServerName  string          `json:"server_name"`
	Mode        string          `json:"mode"`
	ClientDate  string          `json:"client_date"`
	Address     string          `json:"address"`
	LoginPort   int             `json:"login_port"`
	Players     bool            `json:"players"` // false: the portal couldn't read its database
	OnlineCount int             `json:"online_count"`
	Online      []liveCharacter `json:"online"`
	Top         []liveCharacter `json:"top"`
	News        []newsPost      `json:"news"`
	Links       map[string]string
}

type liveCharacter struct {
	Name      string `json:"name"`
	Job       string `json:"job"`
	BaseLevel int    `json:"base_level"`
	JobLevel  int    `json:"job_level"`
	Map       string `json:"map"`
}

type newsPost struct {
	Title string `json:"title"`
	Text  string `json:"text"`
}

// fetchLive gets the server at a glance. Links that would open anything
// but a page on the portal are dropped (safeLink).
func fetchLive(ctx context.Context, portal string) (*liveInfo, error) {
	ctx, cancel := context.WithTimeout(ctx, 10*time.Second)
	defer cancel()
	resp, err := get(ctx, portal+"/patcher/launcher.json")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var raw struct {
		liveInfo
		Links map[string]string `json:"links"`
	}
	if err := json.NewDecoder(io.LimitReader(resp.Body, 4<<20)).Decode(&raw); err != nil {
		return nil, fmt.Errorf("the server's launcher info: %w", err)
	}
	info := raw.liveInfo
	info.Links = map[string]string{}
	for k, v := range raw.Links {
		if u := safeLink(portal, v); u != "" {
			info.Links[k] = u
		}
	}
	return &info, nil
}

// gameUp checks that the game's login server takes connections: the
// address the client logs in to, from this PC.
func gameUp(ctx context.Context, address string, port int) bool {
	if address == "" || port <= 0 {
		return false
	}
	d := net.Dialer{Timeout: 3 * time.Second}
	c, err := d.DialContext(ctx, "tcp", net.JoinHostPort(address, strconv.Itoa(port)))
	if err != nil {
		return false
	}
	c.Close()
	return true
}

// Theme choices.
const (
	themeSystem = "system" // follow Windows' app mode
	themeLight  = "light"
	themeDark   = "dark"
)

// settings is what the player sets on the Settings tab.
type settings struct {
	// KeepOpen leaves the launcher open after Play; by default it closes.
	KeepOpen bool   `json:"keep_open,omitempty"`
	Theme    string `json:"theme,omitempty"`
	// Tab is the tab the launcher was on when it closed.
	Tab string `json:"tab,omitempty"`
	// ServerName is the server's name from last time, shown until the
	// server answers.
	ServerName string `json:"server_name,omitempty"`
}

func loadSettings(dir string) settings {
	var s settings
	if b, err := os.ReadFile(filepath.Join(dir, settingsName)); err == nil {
		json.Unmarshal(b, &s)
	}
	switch s.Theme {
	case themeLight, themeDark:
	default:
		s.Theme = themeSystem
	}
	return s
}

func saveSettings(dir string, s settings) error {
	b, _ := json.MarshalIndent(s, "", "  ")
	return writeFileAtomic(filepath.Join(dir, settingsName), append(b, '\n'))
}

// startRepair makes the next update apply every patch again, from an
// empty rag-wrapper.grf: for a client whose files were damaged or changed
// by hand. Patches only ever write rag-wrapper.grf and their own files in
// the client folder, so nothing else is touched.
func startRepair(dir string) error {
	if err := os.Remove(filepath.Join(dir, defaultGRF)); err != nil && !errors.Is(err, os.ErrNotExist) {
		return inUse(err)
	}
	// 0 isn't a patch number (the desktop app numbers them from 1), so
	// every patch counts as not applied, as after the list starts over.
	return saveState(dir, state{Applied: 0})
}
