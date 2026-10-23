package main

import (
	"archive/zip"
	"bufio"
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"html/template"
	"io"
	"log"
	"maps"
	"net/http"
	"os"
	"path/filepath"
	"regexp"
	"slices"
	"strconv"
	"strings"
	"sync"
	"time"
)

// Players download a client patcher with a config that points at this
// portal, and it fetches the published patches from here: the patched
// client exe first, then custom GRF content and translations. There are
// two, both built for Windows into the portal image: rpatchur
// (github.com/L1nkZ/rpatchur), and our own (cmd/patcher), which replaces
// it once it's been confirmed on players' PCs. Until then rpatchur stays
// the main download and ours is offered beside it for testing.
//
// Patches are THOR archives in the stack's patches/ folder, listed in that
// folder's plist.txt in the order they apply. The portal only serves them;
// they're published from the desktop app.

// Names of the files players get. The yml must share the exe's name: that
// is where rpatchur looks for its config.
const (
	patcherName = "rag-wrapper-patcher"
	patcherZip  = patcherName + ".zip"
	// Our own patcher's download. The exe inside has the same name as
	// rpatchur's, so trying it replaces rpatchur in the client folder.
	newPatcherZip = patcherName + "-new.zip"
	// Our patcher's config; see config in cmd/patcher/patch.go.
	newPatcherConfig = patcherName + ".json"
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
	Dir    string // the stack's patches/ folder: plist.txt and the .thor files
	Exe    string // rpatchur.exe, built into the image; its licence files sit beside it
	NewExe string // our own patcher, built into the image; Go's licence sits beside it as LICENSE-Go
	// Key signs what our patcher downloads (signing.go); nil signs nothing.
	Key ed25519.PrivateKey
}

// patchName is what a patch file in plist.txt may be called. It keeps
// requests inside the patches folder.
var patchName = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._-]*\.thor$`)

// Patches returns the published patch files in plist.txt order. A missing
// plist.txt means nothing is published yet.
func (p patcherFiles) Patches() []string {
	var names []string
	for _, l := range p.listed() {
		names = append(names, l.name)
	}
	return names
}

type listedPatch struct {
	n    int
	name string
}

// listed is plist.txt's patches with their numbers.
func (p patcherFiles) listed() []listedPatch {
	f, err := os.Open(filepath.Join(p.Dir, "plist.txt"))
	if err != nil {
		return nil
	}
	defer f.Close()
	var list []listedPatch
	sc := bufio.NewScanner(f)
	for sc.Scan() {
		// "<number> <file>", the format THOR patchers read.
		fields := strings.Fields(sc.Text())
		if len(fields) < 2 {
			continue
		}
		if n, err := strconv.Atoi(fields[0]); err == nil && patchName.MatchString(fields[1]) {
			list = append(list, listedPatch{n, fields[1]})
		}
	}
	return list
}

// Available is true when the image has the patcher to hand out.
func (p patcherFiles) Available() bool {
	st, err := os.Stat(p.Exe)
	return err == nil && st.Mode().IsRegular()
}

// NewAvailable is true when the image has our own patcher.
func (p patcherFiles) NewAvailable() bool {
	st, err := os.Stat(p.NewExe)
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
	licences := map[string]string{}
	for _, name := range []string{"LICENSE-MIT", "LICENSE-APACHE"} {
		licences[filepath.Join(filepath.Dir(a.conn.Patcher.Exe), name)] = "licenses/rpatchur-" + name + ".txt"
	}
	a.servePatcherZip(w, r, patcherZip, a.conn.Patcher.Exe, map[string][]byte{
		patcherName + ".yml": patcherConfig(base, a.conn.ServerName()),
	}, licences)
}

// newPatcherDownload is the same for our own patcher, whose config only
// says where the portal is: it asks the portal for the rest.
func (a *app) newPatcherDownload(w http.ResponseWriter, r *http.Request) {
	if !a.conn.Patcher.NewAvailable() {
		http.Error(w, "The new launcher isn't in this portal's image. Rebuild the portal with docker compose build portal.", http.StatusNotFound)
		return
	}
	cfg := map[string]string{"portal": a.conn.BaseURL(r)}
	if a.conn.Patcher.Key != nil {
		cfg["public_key"] = publicKey(a.conn.Patcher.Key)
	}
	cfgJSON, _ := json.MarshalIndent(cfg, "", "  ")
	a.servePatcherZip(w, r, newPatcherZip, a.conn.Patcher.NewExe, map[string][]byte{
		newPatcherConfig: append(cfgJSON, '\n'),
	}, map[string]string{
		filepath.Join(filepath.Dir(a.conn.Patcher.NewExe), "LICENSE-Go"):          "licenses/go-LICENSE.txt",
		filepath.Join(filepath.Dir(a.conn.Patcher.NewExe), "LICENSE-third-party"): "licenses/third-party-LICENSES.txt",
	})
}

// servePatcherZip serves a patcher download: the exe as
// rag-wrapper-patcher.exe, extra files, the connection files, and licence
// files (image path to name in the zip) that exist. A built zip is kept
// (patcherZips) and served again while nothing in it changes.
func (a *app) servePatcherZip(w http.ResponseWriter, r *http.Request, zipName, exePath string, extra map[string][]byte, licences map[string]string) {
	b, err := a.patcherZip(r, zipName, exePath, extra, licences)
	if err != nil {
		log.Printf("patcher zip: %v", err)
		http.Error(w, "couldn't build the patcher download", http.StatusInternalServerError)
		return
	}
	w = download(w)
	w.Header().Set("Content-Type", "application/zip")
	w.Header().Set("Content-Disposition", `attachment; filename="`+zipName+`"`)
	w.Header().Set("Cache-Control", "no-store")
	w.Write(b)
}

// zipFile is a small file in a patcher download, beside the exe.
type zipFile struct {
	name string
	data []byte
}

// patcherZip is the download's content. The small files are made on every
// request (the admin IDs come from the database, so a new admin is in the
// next download); the zip is only built again when they or the exe change.
func (a *app) patcherZip(r *http.Request, zipName, exePath string, extra map[string][]byte, licences map[string]string) ([]byte, error) {
	exe, err := os.Open(exePath)
	if err != nil {
		return nil, err
	}
	defer exe.Close()
	// The open file's own size and time, so they match what's read.
	st, err := exe.Stat()
	if err != nil {
		return nil, err
	}
	base := a.conn.BaseURL(r)
	var files []zipFile
	for _, name := range slices.Sorted(maps.Keys(extra)) {
		files = append(files, zipFile{name, extra[name]})
	}
	files = append(files,
		zipFile{"data/clientinfo.xml", a.conn.clientInfo(base+"/register", a.gmAccounts(r.Context()))},
		zipFile{externalSettingsPath, a.conn.externalSettings()})
	for _, src := range slices.Sorted(maps.Keys(licences)) {
		if b, err := os.ReadFile(src); err == nil {
			files = append(files, zipFile{licences[src], b})
		}
	}

	// The key covers everything in the zip: the exe by its path, size and
	// time, and the small files by their content.
	h := sha256.New()
	fmt.Fprintf(h, "%q %q %d %d\n", zipName, exePath, st.Size(), st.ModTime().UnixNano())
	for _, f := range files {
		fmt.Fprintf(h, "%q %d\n", f.name, len(f.data))
		h.Write(f.data)
	}
	var key [sha256.Size]byte
	h.Sum(key[:0])
	return patcherZips.get(key, func() ([]byte, error) {
		var buf bytes.Buffer
		zw := zip.NewWriter(&buf)
		fw, err := zw.Create(patcherName + ".exe")
		if err == nil {
			_, err = io.Copy(fw, exe)
		}
		for _, f := range files {
			if err == nil {
				fw, err = zw.Create(f.name)
			}
			if err == nil {
				_, err = fw.Write(f.data)
			}
		}
		if err == nil {
			err = zw.Close()
		}
		return buf.Bytes(), err
	})
}

// patcherZips keeps the last few patcher downloads built, so a LAN party's
// worth of players downloading at once costs one build, not one each. A
// download differs by the address it was reached on and the zip (rpatchur's
// or ours), so a few entries cover a server.
var patcherZips = &zipCache{max: 4}

type zipCache struct {
	build   sync.Mutex // one build at a time: a rush of downloads builds once
	mu      sync.Mutex // guards entries
	max     int
	entries []zipCacheEntry // least recently used first
}

type zipCacheEntry struct {
	key [sha256.Size]byte
	zip []byte
}

// get returns the zip kept for key, building it with build if there's none.
func (c *zipCache) get(key [sha256.Size]byte, build func() ([]byte, error)) ([]byte, error) {
	if b, ok := c.lookup(key); ok {
		return b, nil
	}
	c.build.Lock()
	defer c.build.Unlock()
	if b, ok := c.lookup(key); ok {
		return b, nil // built while this request waited
	}
	b, err := build()
	if err != nil {
		return nil, err
	}
	c.mu.Lock()
	defer c.mu.Unlock()
	if len(c.entries) >= c.max {
		c.entries = slices.Delete(c.entries, 0, len(c.entries)-c.max+1)
	}
	c.entries = append(c.entries, zipCacheEntry{key, b})
	return b, nil
}

func (c *zipCache) lookup(key [sha256.Size]byte) ([]byte, bool) {
	c.mu.Lock()
	defer c.mu.Unlock()
	for i, e := range c.entries {
		if e.key == key {
			// Move it to the end: the most recently used.
			c.entries = append(slices.Delete(c.entries, i, i+1), e)
			return e.zip, true
		}
	}
	return nil, false
}

// patcherInfo tells our own patcher what it shows and where everything is.
// It's read each time the patcher starts, so the news, the server name and
// the patcher's own updates reach players without a new download.
func (a *app) patcherInfo(w http.ResponseWriter, r *http.Request) {
	base := a.conn.BaseURL(r)
	info := map[string]any{
		"server_name": a.conn.ServerName(),
		"account_url": base + "/account",
		"connect_url": base + "/connect",
		"news_url":    base + "/news",
		"plist_url":   base + "/patcher/plist.txt",
		"patch_url":   base + "/patcher/data/",
		"news":        nil,
	}
	if title, text := a.conn.latestNewsText(); title != "" || text != "" {
		info["news"] = map[string]string{"title": title, "text": text}
	}
	if sum := a.conn.Patcher.newExeSHA256(); sum != "" {
		info["patcher"] = map[string]string{"url": base + "/patcher/" + patcherName + ".exe", "sha256": sum}
	}
	if key := a.conn.Patcher.Key; key != nil {
		info["patches"] = a.conn.Patcher.signedPatches()
		if err := signInfo(key, info); err != nil {
			log.Printf("sign patcher info: %v", err)
			http.Error(w, "couldn't sign the patcher info", http.StatusInternalServerError)
			return
		}
	}
	w.Header().Set("Content-Type", "application/json")
	w.Header().Set("Cache-Control", "no-store")
	json.NewEncoder(w).Encode(info)
}

// newPatcherExe serves our own patcher on its own, for its self-update.
func (a *app) newPatcherExe(w http.ResponseWriter, r *http.Request) {
	if !a.conn.Patcher.NewAvailable() {
		http.NotFound(w, r)
		return
	}
	w = download(w)
	w.Header().Set("Content-Type", "application/octet-stream")
	w.Header().Set("Cache-Control", "no-store")
	http.ServeFile(w, r, a.conn.Patcher.NewExe)
}

func (p patcherFiles) newExeSHA256() string {
	_, sum := fileSum(p.NewExe)
	return sum
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
	w = download(w)
	w.Header().Set("Content-Type", "application/octet-stream")
	http.ServeFile(w, r, filepath.Join(a.conn.Patcher.Dir, name))
}

// downloadStall is how long a download may go without the player taking
// any of it before it's dropped.
var downloadStall = 30 * time.Second

// download lifts the server's WriteTimeout for a file download. That
// timeout covers the whole response, so a patch that takes more than 30
// seconds over Wi-Fi or a VPN was cut off partway, every time. Instead
// each write gets downloadStall to go through, so a slow download
// finishes and only a stalled one is dropped.
func download(w http.ResponseWriter) http.ResponseWriter {
	dw := &downloadWriter{ResponseWriter: w, rc: http.NewResponseController(w)}
	dw.extend()
	return dw
}

type downloadWriter struct {
	http.ResponseWriter
	rc *http.ResponseController
}

func (d *downloadWriter) extend() {
	// Fails only on writers without a connection, such as test recorders.
	d.rc.SetWriteDeadline(time.Now().Add(downloadStall))
}

func (d *downloadWriter) Write(b []byte) (int, error) {
	// Large writes go in pieces, so each piece gets its own deadline.
	const chunk = 64 << 10
	n := 0
	for len(b) > 0 {
		d.extend()
		m, err := d.ResponseWriter.Write(b[:min(len(b), chunk)])
		n += m
		if err != nil {
			return n, err
		}
		b = b[m:]
	}
	return n, nil
}

// Unwrap lets http.ResponseController reach the connection.
func (d *downloadWriter) Unwrap() http.ResponseWriter { return d.ResponseWriter }

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
