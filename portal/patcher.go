package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"encoding/json"
	"html/template"
	"io"
	"log"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"strconv"
	"strings"
)

// The client patcher is rpatchur (github.com/L1nkZ/rpatchur), built for
// Windows into the portal image. Players download it with a config that
// points at this portal, and it fetches the published patches from here:
// the patched client exe first, then custom GRF content and translations.
//
// Patches are THOR archives in the stack's patches/ folder, listed in that
// folder's plist.txt in the order they apply. The portal only serves them;
// they're published from the desktop app.

// Names of the files players get. The yml must share the exe's name: that
// is where rpatchur looks for its config.
const (
	patcherName = "rag-wrapper-patcher"
	patcherZip  = patcherName + ".zip"
)

// The patched client exe the first patch installs, which the patcher's
// Play button starts, and the GRF that later patches write into. Both are
// our own names, so nothing from the official client is overwritten. The
// GRF has to be listed in the client's DATA.INI above data.grf.
const (
	clientExe = "rag-wrapper.exe"
	clientGRF = "rag-wrapper.grf"
)

type patcherFiles struct {
	Dir string // the stack's patches/ folder: plist.txt and the .thor files
	Exe string // rpatchur.exe, built into the image; its licence files sit beside it
}

// patchName is what a patch file in plist.txt may be called. It keeps
// requests inside the patches folder.
var patchName = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]*\.thor$`)

// Patches returns the published patch files in plist.txt order. A missing
// plist.txt means nothing is published yet.
func (p patcherFiles) Patches() []string {
	f, err := os.Open(filepath.Join(p.Dir, "plist.txt"))
	if err != nil {
		return nil
	}
	defer f.Close()
	var names []string
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		// "<number> <file>", the format THOR patchers read.
		fields := strings.Fields(sc.Text())
		if len(fields) < 2 {
			continue
		}
		if _, err := strconv.Atoi(fields[0]); err == nil && patchName.MatchString(fields[1]) {
			names = append(names, fields[1])
		}
	}
	return names
}

// Available is true when the image has the patcher to hand out.
func (p patcherFiles) Available() bool {
	st, err := os.Stat(p.Exe)
	return err == nil && st.Mode().IsRegular()
}

// patcherConfig is rpatchur's config file, with every URL on base, the
// address the player reached the portal on. JSON is valid YAML, and it
// quotes every value safely.
func patcherConfig(base, serverName string) []byte {
	cfg := map[string]any{
		"window": map[string]any{"title": serverName + " patcher", "width": 600, "height": 360, "resizable": false},
		"play":   map[string]any{"path": clientExe, "arguments": []string{}, "exit_on_success": true},
		// The page's Settings button: the client's own settings program,
		// which the WARPGATE client ships as opensetup.exe.
		"setup": map[string]any{"path": "opensetup.exe", "arguments": []string{}, "exit_on_success": false},
		"web": map[string]any{
			"index_url":              base + "/patcher/",
			"preferred_patch_server": serverName,
			"patch_servers": []map[string]string{{
				"name":      serverName,
				"plist_url": base + "/patcher/plist.txt",
				"patch_url": base + "/patcher/data/",
			}},
		},
		"client":   map[string]any{"default_grf_name": clientGRF},
		"patching": map[string]any{"in_place": true, "check_integrity": true, "create_grf": true},
	}
	b, _ := json.MarshalIndent(cfg, "", "  ") // only fails on types it can't encode
	return append(b, '\n')
}

// patcherDownload serves a zip to extract into the client folder: the
// patcher, its config, rpatchur's licences, and the connection and web
// server files in data/, so one download sets up a client.
func (a *app) patcherDownload(w http.ResponseWriter, r *http.Request) {
	if !a.conn.Patcher.Available() {
		http.Error(w, "The patcher isn't in this portal's image. Rebuild the portal with docker compose build portal.", http.StatusNotFound)
		return
	}
	base := a.conn.BaseURL(r)
	var buf bytes.Buffer
	zw := zip.NewWriter(&buf)
	add := func(name string, data []byte) error {
		f, err := zw.Create(name)
		if err == nil {
			_, err = f.Write(data)
		}
		return err
	}
	exe, err := os.ReadFile(a.conn.Patcher.Exe)
	if err == nil {
		err = add(patcherName+".exe", exe)
	}
	if err == nil {
		err = add(patcherName+".yml", patcherConfig(base, a.conn.ServerName()))
	}
	if err == nil {
		err = add("data/clientinfo.xml", a.conn.clientInfo(base+"/register", a.gmAccounts(r.Context())))
	}
	if err == nil {
		err = add(externalSettingsPath, a.conn.externalSettings())
	}
	for _, name := range []string{"LICENSE-MIT", "LICENSE-APACHE"} {
		if b, rerr := os.ReadFile(filepath.Join(filepath.Dir(a.conn.Patcher.Exe), name)); rerr == nil && err == nil {
			err = add("licenses/rpatchur-"+name+".txt", b)
		}
	}
	if err == nil {
		err = zw.Close()
	}
	if err != nil {
		log.Printf("patcher zip: %v", err)
		http.Error(w, "couldn't build the patcher download", http.StatusInternalServerError)
		return
	}
	w.Header().Set("Content-Type", "application/zip")
	w.Header().Set("Content-Disposition", `attachment; filename="`+patcherZip+`"`)
	w.Header().Set("Cache-Control", "no-store")
	w.Write(buf.Bytes())
}

// patcherList serves plist.txt. With nothing published it's empty, which
// the patcher reads as up to date.
func (a *app) patcherList(w http.ResponseWriter, r *http.Request) {
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	f, err := os.Open(filepath.Join(a.conn.Patcher.Dir, "plist.txt"))
	if err != nil {
		return
	}
	defer f.Close()
	io.Copy(w, f)
}

// patchFile serves one .thor patch. Only files listed in plist.txt are
// served, so half-written or withdrawn patches stay private.
func (a *app) patchFile(w http.ResponseWriter, r *http.Request) {
	name := r.PathValue("name")
	listed := false
	for _, p := range a.conn.Patcher.Patches() {
		listed = listed || p == name
	}
	if !listed {
		http.NotFound(w, r)
		return
	}
	w.Header().Set("Content-Type", "application/octet-stream")
	http.ServeFile(w, r, filepath.Join(a.conn.Patcher.Dir, name))
}

// patcherPage is the patcher's window. rpatchur shows it with Windows'
// old IE engine, so the page and its script stay plain (no CSS variables,
// no modern JavaScript).
func (a *app) patcherPage(w http.ResponseWriter, r *http.Request) {
	base := a.conn.BaseURL(r)
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	w.Header().Set("Cache-Control", "no-store")
	w.Header().Set("X-UA-Compatible", "IE=edge")
	err := a.patcherPageTmpl.Execute(w, map[string]any{
		"ServerName": a.conn.ServerName(),
		"Account":    base + "/account",
		"Connect":    base + "/connect",
		"NewsURL":    base + "/news",
		"News":       a.conn.news().Latest,
	})
	if err != nil {
		log.Printf("render patcher: %v", err)
	}
}

func parsePatcherPage() (*template.Template, error) {
	return template.ParseFS(assets, "templates/patcher.html")
}
