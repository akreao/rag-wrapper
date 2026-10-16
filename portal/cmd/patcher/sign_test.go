package main

import (
	"bytes"
	"compress/zlib"
	"context"
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"errors"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var testKey = ed25519.NewKeyFromSeed(bytes.Repeat([]byte{7}, 32))

func pubKey(k ed25519.PrivateKey) string {
	return base64.StdEncoding.EncodeToString(k.Public().(ed25519.PublicKey))
}

// signedPortal serves testdata the way the portal does with a signing key:
// info.json lists the patches in plist with their checksums and is signed
// with key. serve can replace what a patch download returns.
func signedPortal(t *testing.T, plist []string, key ed25519.PrivateKey, serve func(name string, b []byte) []byte) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	var srv *httptest.Server
	mux.HandleFunc("GET /patcher/info.json", func(w http.ResponseWriter, r *http.Request) {
		patches := []map[string]any{}
		for i, name := range plist {
			b, _ := os.ReadFile(filepath.Join("testdata", name))
			sum := sha256.Sum256(b)
			patches = append(patches, map[string]any{"n": i + 1, "name": name, "size": len(b), "sha256": hex.EncodeToString(sum[:])})
		}
		info := map[string]any{
			"server_name": "Test",
			"news":        map[string]string{"title": "Hello", "text": "World"},
			"plist_url":   srv.URL + "/patcher/plist.txt",
			"patch_url":   srv.URL + "/patcher/data/",
			"account_url": srv.URL + "/account",
			"patches":     patches,
			"patcher":     map[string]string{"url": srv.URL + "/patcher/x.exe", "sha256": strings.Repeat("ab", 32)},
		}
		payload, _ := json.Marshal(info)
		// Someone in the middle changing the plain fields changes nothing.
		info["patcher"] = map[string]string{"url": "http://evil/x.exe", "sha256": strings.Repeat("cd", 32)}
		info["public_key"] = pubKey(key)
		info["signed"] = base64.StdEncoding.EncodeToString(payload)
		info["signature"] = base64.StdEncoding.EncodeToString(ed25519.Sign(key, append([]byte(signingContext), payload...)))
		json.NewEncoder(w).Encode(info)
	})
	mux.HandleFunc("GET /patcher/data/{name}", func(w http.ResponseWriter, r *http.Request) {
		b, err := os.ReadFile(filepath.Join("testdata", r.PathValue("name")))
		if err != nil {
			http.NotFound(w, r)
			return
		}
		if serve != nil {
			b = serve(r.PathValue("name"), b)
		}
		w.Write(b)
	})
	srv = httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

func loadInfo(t *testing.T, dir string) (*serverInfo, error) {
	t.Helper()
	cfg, err := loadConfig(dir)
	if err != nil {
		t.Fatal(err)
	}
	return fetchInfo(context.Background(), dir, &cfg)
}

func TestSignedUpdate(t *testing.T) {
	srv := signedPortal(t, []string{"0001_client.thor", "0002_data.thor"}, testKey, nil)
	dir := client(t, srv)
	os.WriteFile(filepath.Join(dir, configName), []byte(`{"portal": "`+srv.URL+`", "public_key": "`+pubKey(testKey)+`"}`), 0o644)
	info, err := loadInfo(t, dir)
	if err != nil {
		t.Fatal(err)
	}
	if !info.signed || info.Patcher.SHA256 != strings.Repeat("ab", 32) || len(info.Patches) != 2 {
		t.Fatalf("info: %+v", info)
	}
	runUpdate(t, dir)
	if got := read(t, filepath.Join(dir, clientExe)); got != "MZ fake client" {
		t.Errorf("exe: %q", got)
	}
	if s := loadState(dir); s.Applied != 2 {
		t.Errorf("state: %+v", s)
	}
}

func TestChangedPatchIsRefused(t *testing.T) {
	// A patch that's a valid THOR, but not the one the server signed.
	other, _ := os.ReadFile("testdata/0004_client.thor")
	srv := signedPortal(t, []string{"0001_client.thor"}, testKey, func(string, []byte) []byte { return other })
	dir := client(t, srv)
	info, err := loadInfo(t, dir)
	if err != nil {
		t.Fatal(err)
	}
	err = update(context.Background(), dir, info, func(string, float64) {})
	var untrusted *trustError
	if !errors.As(err, &untrusted) {
		t.Fatalf("a changed patch: %v", err)
	}
	if !exists(filepath.Join(dir, "old.txt")) || exists(filepath.Join(dir, clientExe)) || exists(filepath.Join(dir, downloadName)) {
		t.Error("the changed patch was applied, or its download left behind")
	}
	if s := loadState(dir); s.Applied != 0 {
		t.Errorf("state: %+v", s)
	}
}

// A patcher set up before signing takes the server's key the first time
// and holds the server to it from then on.
func TestKeyIsKept(t *testing.T) {
	srv := signedPortal(t, []string{"0001_client.thor"}, testKey, nil)
	dir := client(t, srv)
	if _, err := loadInfo(t, dir); err != nil {
		t.Fatal(err)
	}
	cfg, err := loadConfig(dir)
	if err != nil || cfg.PublicKey != pubKey(testKey) || cfg.Portal != srv.URL {
		t.Fatalf("config: %+v %v", cfg, err)
	}

	// Another key.
	other := ed25519.NewKeyFromSeed(bytes.Repeat([]byte{8}, 32))
	srv2 := signedPortal(t, []string{"0001_client.thor"}, other, nil)
	os.WriteFile(filepath.Join(dir, configName), []byte(`{"portal": "`+srv2.URL+`", "public_key": "`+pubKey(testKey)+`"}`), 0o644)
	var untrusted *trustError
	if _, err := loadInfo(t, dir); !errors.As(err, &untrusted) || !strings.Contains(err.Error(), "key has changed") {
		t.Errorf("another key: %v", err)
	}

	// No signature at all, as from the old portal.
	srv3 := portal(t, "1 0001_client.thor\n")
	os.WriteFile(filepath.Join(dir, configName), []byte(`{"portal": "`+srv3.URL+`", "public_key": "`+pubKey(testKey)+`"}`), 0o644)
	if _, err := loadInfo(t, dir); !errors.As(err, &untrusted) || !strings.Contains(err.Error(), "isn't signed") {
		t.Errorf("unsigned: %v", err)
	}
}

func TestBadSignature(t *testing.T) {
	raw := &serverInfo{PublicKey: pubKey(testKey)}
	payload := []byte(`{"patch_url":"http://x/","patches":[]}`)
	raw.Signed = base64.StdEncoding.EncodeToString(payload)
	raw.Signature = base64.StdEncoding.EncodeToString(ed25519.Sign(testKey, append([]byte(signingContext), payload...)))
	if _, err := verify(pubKey(testKey), raw); err != nil {
		t.Fatal(err)
	}
	// Signed without the context: not the portal's signature for info.json.
	bad := *raw
	bad.Signature = base64.StdEncoding.EncodeToString(ed25519.Sign(testKey, payload))
	if _, err := verify(pubKey(testKey), &bad); err == nil {
		t.Error("a signature without the context passed")
	}
	bad = *raw
	bad.Signed = base64.StdEncoding.EncodeToString([]byte(`{"patch_url":"http://evil/","patches":[]}`))
	if _, err := verify(pubKey(testKey), &bad); err == nil {
		t.Error("a changed payload passed")
	}
	if _, err := verify("short", raw); err == nil {
		t.Error("a damaged key passed")
	}
}

func TestSafeLink(t *testing.T) {
	portal := "http://10.0.0.5:8080"
	for u, want := range map[string]string{
		"http://10.0.0.5:8080/account":   "http://10.0.0.5:8080/account",
		"https://10.0.0.5:8080/news":     "https://10.0.0.5:8080/news",
		"http://10.0.0.6:8080/account":   "",
		"http://10.0.0.5:9999/account":   "",
		`\\10.0.0.5\share\x.exe`:         "",
		`C:\Windows\System32\cmd.exe`:    "",
		"file:///C:/Windows/notepad.exe": "",
		"http://a@10.0.0.5:8080/":        "",
		"":                               "",
	} {
		if got := safeLink(portal, u); got != want {
			t.Errorf("%q: %q", u, got)
		}
	}
}

// thorPatch writes a client-folder THOR patch of the given files.
func thorPatch(files map[string]string) []byte {
	var data, table bytes.Buffer
	head := []byte(thorMagic)
	head = append(head, 0)
	head = binary.LittleEndian.AppendUint32(head, uint32(len(files)))
	head = binary.LittleEndian.AppendUint16(head, thorMultipleFiles)
	head = append(head, 0)
	start := len(head) + 8
	for name, content := range files {
		var z bytes.Buffer
		zw := zlib.NewWriter(&z)
		zw.Write([]byte(content))
		zw.Close()
		table.WriteByte(byte(len(name)))
		table.WriteString(name)
		table.WriteByte(0)
		table.Write(binary.LittleEndian.AppendUint32(nil, uint32(start+data.Len())))
		table.Write(binary.LittleEndian.AppendUint32(nil, uint32(z.Len())))
		table.Write(binary.LittleEndian.AppendUint32(nil, uint32(len(content))))
		data.Write(z.Bytes())
	}
	var zt bytes.Buffer
	zw := zlib.NewWriter(&zt)
	zw.Write(table.Bytes())
	zw.Close()
	head = binary.LittleEndian.AppendUint32(head, uint32(zt.Len()))
	head = binary.LittleEndian.AppendUint32(head, uint32(start+data.Len()))
	return append(append(head, data.Bytes()...), zt.Bytes()...)
}

func TestIntegrityIsRequired(t *testing.T) {
	check := func(files map[string]string) error {
		b := thorPatch(files)
		a, err := readThor(bytes.NewReader(b), int64(len(b)))
		if err != nil {
			t.Fatal(err)
		}
		return a.checkIntegrity()
	}
	if err := check(map[string]string{"a.txt": "a"}); err == nil || !strings.Contains(err.Error(), "no data.integrity") {
		t.Errorf("no data.integrity: %v", err)
	}
	if err := check(map[string]string{"a.txt": "a", "b.txt": "b", integrityFile: "a.txt=0xe8b7be43\r\n"}); err == nil || !strings.Contains(err.Error(), "b.txt isn't listed") {
		t.Errorf("b.txt unlisted: %v", err)
	}
	if err := check(map[string]string{"a.txt": "a", integrityFile: "a.txt=0xe8b7be43\r\n"}); err != nil {
		t.Errorf("all listed: %v", err)
	}
}
