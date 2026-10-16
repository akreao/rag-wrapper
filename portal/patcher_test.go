package main

import (
	"archive/zip"
	"bytes"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"testing"
	"time"
)

// patcherSetup makes a patches folder with plist.txt and a fake patcher
// exe, and returns a portal that uses them.
func patcherSetup(t *testing.T, plist string) (http.Handler, string) {
	t.Helper()
	dir := t.TempDir()
	patches := filepath.Join(dir, "patches")
	os.Mkdir(patches, 0o755)
	if plist != "" {
		os.WriteFile(filepath.Join(patches, "plist.txt"), []byte(plist), 0o644)
	}
	os.WriteFile(filepath.Join(patches, "0001_client.thor"), []byte("THOR"), 0o644)
	os.WriteFile(filepath.Join(patches, "0002_draft.thor"), []byte("DRAFT"), 0o644)
	os.WriteFile(filepath.Join(dir, "rpatchur.exe"), []byte("MZ"), 0o644)
	os.WriteFile(filepath.Join(dir, "LICENSE-MIT"), []byte("MIT"), 0o644)
	os.WriteFile(filepath.Join(dir, "rag-wrapper-patcher.exe"), []byte("MZ new"), 0o644)
	os.WriteFile(filepath.Join(dir, "LICENSE-Go"), []byte("BSD"), 0o644)
	a, err := newApp(nil, 99, connectInfo{
		Address: "10.0.0.5", PacketVer: "20250716", Overrides: dir,
		Patcher: patcherFiles{Dir: patches, Exe: filepath.Join(dir, "rpatchur.exe"), NewExe: filepath.Join(dir, "rag-wrapper-patcher.exe")},
	})
	if err != nil {
		t.Fatal(err)
	}
	return a.routes(), dir
}

func get(h http.Handler, url string) *httptest.ResponseRecorder {
	rec := httptest.NewRecorder()
	h.ServeHTTP(rec, httptest.NewRequest(http.MethodGet, url, nil))
	return rec
}

func TestPatches(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "plist.txt"), []byte("// comment\n1 0001_client.thor\n\n2 ../secret.thor\nx bad.thor\n3 0003_items.thor\n"), 0o644)
	got := patcherFiles{Dir: dir}.Patches()
	if len(got) != 2 || got[0] != "0001_client.thor" || got[1] != "0003_items.thor" {
		t.Fatal(got)
	}
	if got := (patcherFiles{Dir: t.TempDir()}).Patches(); got != nil {
		t.Fatal(got)
	}
}

func TestPatcherConfig(t *testing.T) {
	var cfg struct {
		Play  struct{ Path string }
		Setup struct{ Path string }
		Web   struct {
			IndexURL     string `json:"index_url"`
			PatchServers []struct {
				PlistURL string `json:"plist_url"`
				PatchURL string `json:"patch_url"`
			} `json:"patch_servers"`
		}
		Client struct {
			DefaultGRFName string `json:"default_grf_name"`
		}
	}
	if err := json.Unmarshal(patcherConfig("http://10.0.0.5:8080", `My "LAN"`), &cfg); err != nil {
		t.Fatal(err)
	}
	if cfg.Play.Path != clientExe || cfg.Setup.Path != "opensetup.exe" || cfg.Web.IndexURL != "http://10.0.0.5:8080/patcher/" || len(cfg.Web.PatchServers) != 1 ||
		cfg.Web.PatchServers[0].PlistURL != "http://10.0.0.5:8080/patcher/plist.txt" ||
		cfg.Web.PatchServers[0].PatchURL != "http://10.0.0.5:8080/patcher/data/" || cfg.Client.DefaultGRFName != clientGRF {
		t.Fatalf("%+v", cfg)
	}
}

func TestPatcherRoutes(t *testing.T) {
	h, _ := patcherSetup(t, "1 0001_client.thor\n")

	if rec := get(h, "/patcher/plist.txt"); rec.Code != http.StatusOK || rec.Body.String() != "1 0001_client.thor\n" {
		t.Fatalf("plist: %d %q", rec.Code, rec.Body)
	}
	if rec := get(h, "/patcher/data/0001_client.thor"); rec.Code != http.StatusOK || rec.Body.String() != "THOR" {
		t.Fatalf("listed patch: %d %q", rec.Code, rec.Body)
	}
	for _, url := range []string{"/patcher/data/0002_draft.thor", "/patcher/data/plist.txt", "/patcher/data/..%2frpatchur.exe"} {
		if rec := get(h, url); rec.Code != http.StatusNotFound {
			t.Errorf("%s: %d", url, rec.Code)
		}
	}

	rec := get(h, "/patcher/")
	if rec.Code != http.StatusOK || !strings.Contains(rec.Body.String(), "/static/patcher.js") ||
		!strings.Contains(rec.Body.String(), `href="http://example.com/account"`) {
		t.Fatalf("patcher page: %d\n%s", rec.Code, rec.Body)
	}

	rec = get(h, "http://10.0.0.5:8080/connect/"+patcherZip)
	if rec.Code != http.StatusOK || rec.Header().Get("Content-Type") != "application/zip" {
		t.Fatalf("zip: %d %v", rec.Code, rec.Header())
	}
	zr, err := zip.NewReader(bytes.NewReader(rec.Body.Bytes()), int64(rec.Body.Len()))
	if err != nil {
		t.Fatal(err)
	}
	files := map[string]string{}
	for _, f := range zr.File {
		r, _ := f.Open()
		b, _ := io.ReadAll(r)
		files[f.Name] = string(b)
	}
	if files[patcherName+".exe"] != "MZ" || files["licenses/rpatchur-LICENSE-MIT.txt"] != "MIT" ||
		!strings.Contains(files[patcherName+".yml"], `"http://10.0.0.5:8080/patcher/plist.txt"`) ||
		!strings.Contains(files["data/clientinfo.xml"], "<address>10.0.0.5</address>") || files["data/sclientinfo.xml"] != "" ||
		!strings.Contains(files[externalSettingsPath], `AssistAddr = "10.0.0.5:8888"`) {
		t.Fatalf("zip files: %v", files)
	}

	rec = get(h, "/connect")
	if body := rec.Body.String(); !strings.Contains(body, "Download the patcher") || !strings.Contains(body, "<em>LANG</em>") ||
		strings.Contains(body, "WARP0716") || !strings.Contains(body, `href="/connect/`+newPatcherZip+`"`) ||
		!strings.Contains(body, "run WARPGATE.exe") || !strings.Contains(body, "the one that has been checked under Wine") {
		t.Fatalf("connect with a patch published:\n%s", body)
	}
}

func unzip(t *testing.T, rec *httptest.ResponseRecorder) map[string]string {
	t.Helper()
	zr, err := zip.NewReader(bytes.NewReader(rec.Body.Bytes()), int64(rec.Body.Len()))
	if err != nil {
		t.Fatalf("%d %v", rec.Code, err)
	}
	files := map[string]string{}
	for _, f := range zr.File {
		r, _ := f.Open()
		b, _ := io.ReadAll(r)
		files[f.Name] = string(b)
	}
	return files
}

// Our own patcher (cmd/patcher): its download, the info it starts with,
// and the exe it updates itself from.
func TestNewPatcherRoutes(t *testing.T) {
	h, dir := patcherSetup(t, "1 0001_client.thor\n")
	os.MkdirAll(filepath.Join(dir, "portal"), 0o755)
	os.WriteFile(filepath.Join(dir, "portal", "news.md"), []byte("# News\n\n## Maintenance tonight\n\nThe server is down at **8pm**, see [the forum](http://x).\n\n- one\n- two\n\n## Older\n\nold\n"), 0o644)
	os.WriteFile(filepath.Join(dir, "char_conf.txt"), []byte("server_name: LAN RO\n"), 0o644)

	files := unzip(t, get(h, "http://localhost:8080/connect/"+newPatcherZip))
	if files[patcherName+".exe"] != "MZ new" || files["licenses/go-LICENSE.txt"] != "BSD" || files[patcherName+".yml"] != "" ||
		!strings.Contains(files[newPatcherConfig], `"portal": "http://10.0.0.5:8080"`) ||
		!strings.Contains(files["data/clientinfo.xml"], "<address>10.0.0.5</address>") ||
		!strings.Contains(files[externalSettingsPath], `AssistAddr = "10.0.0.5:8888"`) {
		t.Fatalf("zip files: %v", files)
	}

	rec := get(h, "http://10.0.0.5:8080/patcher/info.json")
	var info struct {
		ServerName string `json:"server_name"`
		News       struct{ Title, Text string }
		PlistURL   string `json:"plist_url"`
		PatchURL   string `json:"patch_url"`
		AccountURL string `json:"account_url"`
		Patcher    struct{ URL, SHA256 string }
	}
	if err := json.Unmarshal(rec.Body.Bytes(), &info); err != nil {
		t.Fatal(err)
	}
	sum := sha256.Sum256([]byte("MZ new"))
	if info.ServerName != "LAN RO" || info.News.Title != "Maintenance tonight" ||
		info.News.Text != "The server is down at 8pm, see the forum.\n\n• one\n• two" ||
		info.PlistURL != "http://10.0.0.5:8080/patcher/plist.txt" || info.PatchURL != "http://10.0.0.5:8080/patcher/data/" ||
		info.AccountURL != "http://10.0.0.5:8080/account" ||
		info.Patcher.URL != "http://10.0.0.5:8080/patcher/rag-wrapper-patcher.exe" || info.Patcher.SHA256 != hex.EncodeToString(sum[:]) {
		t.Fatalf("info: %s", rec.Body)
	}
	if rec := get(h, "/patcher/rag-wrapper-patcher.exe"); rec.Code != http.StatusOK || rec.Body.String() != "MZ new" {
		t.Fatalf("exe: %d", rec.Code)
	}

	// A new build is picked up.
	os.WriteFile(filepath.Join(dir, "rag-wrapper-patcher.exe"), []byte("MZ newer build"), 0o644)
	os.Chtimes(filepath.Join(dir, "rag-wrapper-patcher.exe"), time.Now().Add(time.Hour), time.Now().Add(time.Hour))
	if !strings.Contains(get(h, "/patcher/info.json").Body.String(), func() string { s := sha256.Sum256([]byte("MZ newer build")); return hex.EncodeToString(s[:]) }()) {
		t.Error("info.json kept the old checksum")
	}
}

func TestNewPatcherMissing(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20250716", Overrides: t.TempDir(),
		Patcher: patcherFiles{Dir: t.TempDir(), Exe: "/nonexistent/rpatchur.exe", NewExe: "/nonexistent/new.exe"}})
	if err != nil {
		t.Fatal(err)
	}
	h := a.routes()
	for _, url := range []string{"/connect/" + newPatcherZip, "/patcher/rag-wrapper-patcher.exe"} {
		if rec := get(h, url); rec.Code != http.StatusNotFound {
			t.Errorf("%s: %d", url, rec.Code)
		}
	}
	// No news and no patcher: the info still lists the patches.
	body := get(h, "/patcher/info.json").Body.String()
	if !strings.Contains(body, `"news":null`) || strings.Contains(body, "sha256") || !strings.Contains(body, "plist_url") {
		t.Errorf("info: %s", body)
	}
}

func TestNewsText(t *testing.T) {
	dir := t.TempDir()
	os.MkdirAll(filepath.Join(dir, "portal"), 0o755)
	os.WriteFile(filepath.Join(dir, "portal", "news.md"), []byte("## Patch 2\n\n1. first\n2. second\n\n> quoted *text*\n\n```\ncode line\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n\nline one  \nline two\n"), 0o644)
	title, text := connectInfo{Overrides: dir}.latestNewsText()
	want := "1. first\n2. second\n\n> quoted text\n\n    code line\n\na | b\n1 | 2\n\nline one\nline two"
	if title != "Patch 2" || text != want {
		t.Errorf("title %q\ntext %q\nwant %q", title, text, want)
	}
}

func TestConnectWithoutPatches(t *testing.T) {
	h, _ := patcherSetup(t, "")
	if body := get(h, "/connect").Body.String(); strings.Contains(body, "Download the patcher") || !strings.Contains(body, "WARP0716") ||
		!strings.Contains(body, "<em>LANG</em>") {
		t.Fatalf("connect with nothing published:\n%s", body)
	}
	if rec := get(h, "/patcher/plist.txt"); rec.Code != http.StatusOK || rec.Body.Len() != 0 {
		t.Fatalf("empty plist: %d %q", rec.Code, rec.Body)
	}
}

func TestPatcherMissing(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Address: "10.0.0.5", PacketVer: "20250716",
		Patcher: patcherFiles{Dir: t.TempDir(), Exe: "/nonexistent/rpatchur.exe"}})
	if err != nil {
		t.Fatal(err)
	}
	if rec := get(a.routes(), "/connect/"+patcherZip); rec.Code != http.StatusNotFound {
		t.Fatal(rec.Code)
	}
}

// Downloading on the server itself mustn't hand players a config that
// points at their own PC.
func TestPatcherConfigFromLocalhost(t *testing.T) {
	h, _ := patcherSetup(t, "1 0001_client.thor\n")
	for _, host := range []string{"localhost:8080", "127.0.0.1:8080", "[::1]:8080"} {
		rec := get(h, "http://"+host+"/connect/"+patcherZip)
		zr, err := zip.NewReader(bytes.NewReader(rec.Body.Bytes()), int64(rec.Body.Len()))
		if err != nil {
			t.Fatalf("%s: %v", host, err)
		}
		for _, f := range zr.File {
			if f.Name != patcherName+".yml" {
				continue
			}
			r, _ := f.Open()
			b, _ := io.ReadAll(r)
			if !strings.Contains(string(b), `"http://10.0.0.5:8080/patcher/plist.txt"`) || strings.Contains(string(b), host) {
				t.Errorf("%s: config\n%s", host, b)
			}
		}
	}
	// A server that only advertises loopback keeps the address it was reached on.
	if got := (connectInfo{Address: "127.0.0.1"}).BaseURL(httptest.NewRequest(http.MethodGet, "http://localhost:8080/", nil)); got != "http://localhost:8080" {
		t.Errorf("loopback server: %s", got)
	}
}

// With PATCH_SIGNING_KEY set, info.json is signed and lists each patch's
// checksum, and the patcher download carries the key to check it with.
func TestSignedPatcherInfo(t *testing.T) {
	dir := t.TempDir()
	patches := filepath.Join(dir, "patches")
	os.Mkdir(patches, 0o755)
	os.WriteFile(filepath.Join(patches, "plist.txt"), []byte("1 0001_client.thor\n3 0003_data.thor\n"), 0o644)
	os.WriteFile(filepath.Join(patches, "0001_client.thor"), []byte("THOR"), 0o644)
	os.WriteFile(filepath.Join(dir, "rag-wrapper-patcher.exe"), []byte("MZ new"), 0o644)
	key := signingKey("secret")
	a, err := newApp(nil, 99, connectInfo{
		Address: "10.0.0.5", PacketVer: "20250716", Overrides: dir,
		Patcher: patcherFiles{Dir: patches, Exe: "/nonexistent", NewExe: filepath.Join(dir, "rag-wrapper-patcher.exe"), Key: key},
	})
	if err != nil {
		t.Fatal(err)
	}
	h := a.routes()

	files := unzip(t, get(h, "http://10.0.0.5:8080/connect/"+newPatcherZip))
	var cfg struct{ Portal, Public_Key string }
	if err := json.Unmarshal([]byte(files[newPatcherConfig]), &cfg); err != nil || cfg.Public_Key != publicKey(key) {
		t.Fatalf("config: %s", files[newPatcherConfig])
	}

	var info struct {
		ServerName string `json:"server_name"`
		PublicKey  string `json:"public_key"`
		Signed     string
		Signature  string
	}
	body := get(h, "http://10.0.0.5:8080/patcher/info.json").Body.Bytes()
	if err := json.Unmarshal(body, &info); err != nil || info.PublicKey != publicKey(key) {
		t.Fatalf("info: %s", body)
	}
	payload, _ := base64.StdEncoding.DecodeString(info.Signed)
	sig, _ := base64.StdEncoding.DecodeString(info.Signature)
	pub, _ := base64.StdEncoding.DecodeString(cfg.Public_Key)
	if !ed25519.Verify(pub, append([]byte(signingContext), payload...), sig) {
		t.Fatal("the signature doesn't verify")
	}
	var signed struct {
		ServerName string `json:"server_name"`
		Patches    []signedPatch
		Patcher    struct{ SHA256 string }
	}
	json.Unmarshal(payload, &signed)
	thor, exe := sha256.Sum256([]byte("THOR")), sha256.Sum256([]byte("MZ new"))
	// A listed patch that's missing has no checksum; the patcher stops there.
	want := []signedPatch{{1, "0001_client.thor", 4, hex.EncodeToString(thor[:])}, {3, "0003_data.thor", 0, ""}}
	if signed.ServerName != info.ServerName || !slices.Equal(signed.Patches, want) || signed.Patcher.SHA256 != hex.EncodeToString(exe[:]) {
		t.Fatalf("signed: %s", payload)
	}

	// The patcher checks with the same context.
	src, _ := os.ReadFile("cmd/patcher/sign.go")
	if !strings.Contains(string(src), "const signingContext = "+strconv.Quote(signingContext)) {
		t.Error("cmd/patcher's signingContext differs from the portal's")
	}

	// Without a key nothing is signed, as before.
	if signingKey("") != nil {
		t.Error("an empty PATCH_SIGNING_KEY made a key")
	}
}
