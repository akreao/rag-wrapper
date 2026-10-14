package main

import (
	"archive/zip"
	"bytes"
	"encoding/json"
	"io"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
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
	a, err := newApp(nil, 99, connectInfo{
		Address: "10.0.0.5", PacketVer: "20250716", Overrides: dir,
		Patcher: patcherFiles{Dir: patches, Exe: filepath.Join(dir, "rpatchur.exe")},
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
		Play struct{ Path string }
		Web  struct {
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
	if cfg.Play.Path != clientExe || cfg.Web.IndexURL != "http://10.0.0.5:8080/patcher/" || len(cfg.Web.PatchServers) != 1 ||
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
		!strings.Contains(files["data/clientinfo.xml"], "<address>10.0.0.5</address>") || files["data/sclientinfo.xml"] != "" {
		t.Fatalf("zip files: %v", files)
	}

	rec = get(h, "/connect")
	if body := rec.Body.String(); !strings.Contains(body, "Download the patcher") || !strings.Contains(body, "<em>LANG</em>") ||
		strings.Contains(body, "WARP0716") {
		t.Fatalf("connect with a patch published:\n%s", body)
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
