package main

import (
	"bytes"
	"context"
	"encoding/json"
	"hash/crc32"
	"math/rand"
	"net/http"
	"net/http/httptest"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

// The patches in testdata were written by the desktop app's THOR writer
// (desktop/src-tauri/src/thor.rs), as a first publish and a later one:
//   - 0001_client.thor: rag-wrapper.exe, System\itemInfo_C.lua, DATA.INI,
//     and a removal of old.txt
//   - 0002_data.thor: four files into rag-wrapper.grf, one with a CP949 name
//   - 0003_data.thor: one of them changed, one removed
//   - 0004_client.thor: System\itemInfo_C.lua removed

// portal serves testdata the way the portal does, with plist as plist.txt.
func portal(t *testing.T, plist string) *httptest.Server {
	t.Helper()
	mux := http.NewServeMux()
	var srv *httptest.Server
	mux.HandleFunc("GET /patcher/info.json", func(w http.ResponseWriter, r *http.Request) {
		json.NewEncoder(w).Encode(map[string]any{
			"server_name": "Test",
			"news":        map[string]string{"title": "Hello", "text": "World"},
			"plist_url":   srv.URL + "/patcher/plist.txt",
			"patch_url":   srv.URL + "/patcher/data/",
		})
	})
	mux.HandleFunc("GET /patcher/plist.txt", func(w http.ResponseWriter, r *http.Request) { w.Write([]byte(plist)) })
	mux.Handle("GET /patcher/data/", http.StripPrefix("/patcher/data/", http.FileServer(http.Dir("testdata"))))
	srv = httptest.NewServer(mux)
	t.Cleanup(srv.Close)
	return srv
}

func client(t *testing.T, srv *httptest.Server) string {
	t.Helper()
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, configName), []byte(`{"portal": "`+srv.URL+`/"}`), 0o644)
	os.WriteFile(filepath.Join(dir, "data.grf"), []byte("official"), 0o644)
	os.WriteFile(filepath.Join(dir, "old.txt"), []byte("old"), 0o644)
	return dir
}

func runUpdate(t *testing.T, dir string) {
	t.Helper()
	cfg, err := loadConfig(dir)
	if err != nil {
		t.Fatal(err)
	}
	info, err := fetchInfo(context.Background(), dir, &cfg)
	if err != nil {
		t.Fatal(err)
	}
	if info.ServerName != "Test" || info.News == nil || info.News.Title != "Hello" {
		t.Fatalf("info: %+v", info)
	}
	var last float64
	err = update(context.Background(), dir, info, func(text string, f float64) {
		if f < last || f > 1 {
			t.Errorf("progress went from %v to %v (%s)", last, f, text)
		}
		last = f
	})
	if err != nil {
		t.Fatal(err)
	}
	if last != 1 {
		t.Errorf("progress ended at %v", last)
	}
}

// grfFiles reads every file in a GRF.
func grfFiles(t *testing.T, path string) map[string]string {
	t.Helper()
	g, err := openGRF(path)
	if err != nil {
		t.Fatal(err)
	}
	defer g.close()
	files := map[string]string{}
	for _, e := range g.entries {
		b, err := inflate(grfSection(g, e), int64(e.Size))
		if err != nil || len(b) != int(e.Size) || e.Flags != grfFlagFile {
			t.Fatalf("%s: %v (%d bytes, flags %d)", e.Name, err, len(b), e.Flags)
		}
		files[e.Name] = string(b)
	}
	return files
}

func read(t *testing.T, path string) string {
	t.Helper()
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	return string(b)
}

func exists(path string) bool {
	_, err := os.Stat(path)
	return err == nil
}

func TestUpdateAppliesPublishedPatches(t *testing.T) {
	plist := "1 0001_client.thor\n2 0002_data.thor\n"
	srv := portal(t, plist)
	dir := client(t, srv)
	runUpdate(t, dir)

	if got := read(t, filepath.Join(dir, clientExe)); got != "MZ fake client" {
		t.Errorf("exe: %q", got)
	}
	if got := read(t, filepath.Join(dir, "System", "itemInfo_C.lua")); got != "tbl_custom = {}\n" {
		t.Errorf("lua: %q", got)
	}
	if got := read(t, filepath.Join(dir, "DATA.INI")); !strings.Contains(got, "0=rag-wrapper.grf") {
		t.Errorf("DATA.INI: %q", got)
	}
	if exists(filepath.Join(dir, "old.txt")) || exists(filepath.Join(dir, integrityFile)) || exists(filepath.Join(dir, downloadName)) {
		t.Error("old.txt, data.integrity or the download is still in the client folder")
	}
	if got := read(t, filepath.Join(dir, "data.grf")); got != "official" {
		t.Error("data.grf changed")
	}
	files := grfFiles(t, filepath.Join(dir, defaultGRF))
	want := map[string]string{
		`data\texture\custom.bmp`:            strings.Repeat("\x07", 5000),
		"data\\sprite\\\xc0\xaf\xc0\xfa.spr": "korean name",
		`data\gone.txt`:                      "to be removed",
		`data\empty.txt`:                     "",
	}
	if len(files) != len(want) {
		t.Fatalf("GRF: %q", files)
	}
	for k, v := range want {
		if files[k] != v {
			t.Errorf("GRF %q: %q", k, files[k])
		}
	}
	if s := loadState(dir); s.Applied != 2 {
		t.Errorf("state: %+v", s)
	}

	// Nothing new: nothing changes.
	before := read(t, filepath.Join(dir, defaultGRF))
	runUpdate(t, dir)
	if read(t, filepath.Join(dir, defaultGRF)) != before {
		t.Error("an update with nothing new changed the GRF")
	}

	// The next publish.
	srv2 := portal(t, plist+"3 0003_data.thor\n4 0004_client.thor\n")
	os.WriteFile(filepath.Join(dir, configName), []byte(`{"portal": "`+srv2.URL+`"}`), 0o644)
	runUpdate(t, dir)
	files = grfFiles(t, filepath.Join(dir, defaultGRF))
	if files[`data\texture\custom.bmp`] != "changed" || len(files) != 3 {
		t.Errorf("GRF after the second publish: %q", files)
	}
	if _, ok := files[`data\gone.txt`]; ok {
		t.Error("data\\gone.txt is still in the GRF")
	}
	if exists(filepath.Join(dir, "System", "itemInfo_C.lua")) {
		t.Error("the removed lua is still there")
	}
	if s := loadState(dir); s.Applied != 4 {
		t.Errorf("state: %+v", s)
	}
}

func TestDamagedPatchIsNotApplied(t *testing.T) {
	b, _ := os.ReadFile("testdata/0002_data.thor")
	a, err := readThor(bytes.NewReader(b), int64(len(b)))
	if err != nil {
		t.Fatal(err)
	}
	// Corrupt one byte of the first file's compressed data so it still
	// inflates (stored data would) or fails: either way it's refused.
	e := a.Entries[0]
	b[e.Offset+int64(e.Compressed)-5] ^= 0xff
	a, err = readThor(bytes.NewReader(b), int64(len(b)))
	if err == nil {
		err = a.checkIntegrity()
	}
	if err == nil {
		t.Fatal("a damaged patch passed its integrity check")
	}

	dir := t.TempDir()
	path := filepath.Join(dir, "x.thor")
	os.WriteFile(path, b, 0o644)
	if err := applyFile(dir, path); err == nil {
		t.Fatal("applied a damaged patch")
	}
	if exists(filepath.Join(dir, defaultGRF)) {
		t.Error("a damaged patch created the GRF")
	}
	if _, err := readThor(bytes.NewReader(b[:40]), 40); err == nil {
		t.Error("read a truncated patch")
	}
	if _, err := readThor(bytes.NewReader([]byte("<html>")), 6); err != errNotThor {
		t.Errorf("an HTML page: %v", err)
	}
}

func TestIntegrityMatchesTheWriter(t *testing.T) {
	b, _ := os.ReadFile("testdata/0001_client.thor")
	a, err := readThor(bytes.NewReader(b), int64(len(b)))
	if err != nil {
		t.Fatal(err)
	}
	if a.GRFMerging || a.GRF != "" {
		t.Errorf("client patch: merging %v, GRF %q", a.GRFMerging, a.GRF)
	}
	for _, e := range a.Entries {
		if e.Path == integrityFile {
			c, _ := a.content(e)
			want := "rag-wrapper.exe=0x" + hex8(crc32.ChecksumIEEE([]byte("MZ fake client")))
			if !strings.HasPrefix(string(c), want+"\r\n") {
				t.Errorf("data.integrity: %q", c)
			}
		}
	}
	b, _ = os.ReadFile("testdata/0002_data.thor")
	if a, _ = readThor(bytes.NewReader(b), int64(len(b))); !a.GRFMerging || a.GRF != defaultGRF {
		t.Errorf("data patch: merging %v, GRF %q", a.GRFMerging, a.GRF)
	}
}

func TestPending(t *testing.T) {
	list := []plistEntry{{1, "a"}, {2, "b"}, {5, "c"}}
	if got := pending(list, 2); len(got) != 1 || got[0].N != 5 {
		t.Errorf("after 2: %v", got)
	}
	if got := pending(list, 5); len(got) != 0 {
		t.Errorf("after 5: %v", got)
	}
	// A number that isn't in the list: the server started its list over,
	// so everything applies again (as in rpatchur).
	for _, applied := range []int{0, 3, 9} {
		if got := pending(list, applied); len(got) != 3 {
			t.Errorf("after %d: %v", applied, got)
		}
	}
}

// rpatchur's cache file carries over, so switching patchers doesn't
// download everything again.
func TestStateIsRpatchurCache(t *testing.T) {
	dir := t.TempDir()
	os.WriteFile(filepath.Join(dir, "rag-wrapper-patcher.dat"), []byte(`{"last_patch_index":7}`), 0o644)
	if s := loadState(dir); s.Applied != 7 {
		t.Fatalf("%+v", s)
	}
	saveState(dir, state{Applied: 8})
	if got := read(t, filepath.Join(dir, "rag-wrapper-patcher.dat")); got != `{"last_patch_index":8}` {
		t.Errorf("%s", got)
	}
}

func TestParsePlist(t *testing.T) {
	got, _ := parsePlist(strings.NewReader("// comment\n1 0001_client.thor\n\n2 ../x.thor\nx bad.thor\n3 0003_data.thor extra\n"))
	if len(got) != 2 || got[0] != (plistEntry{1, "0001_client.thor"}) || got[1] != (plistEntry{3, "0003_data.thor"}) {
		t.Errorf("%v", got)
	}
}

func TestClientPath(t *testing.T) {
	dir := t.TempDir()
	if p, err := clientPath(dir, `System\a.lub`); err != nil || p != filepath.Join(dir, "System", "a.lub") {
		t.Errorf("%q %v", p, err)
	}
	for _, bad := range []string{``, `\x`, `/x`, `..\x`, `a\..\..\x`, `C:\x`, `a\\b`, `a\.\b`, "caf\xe9", configName, "RAG-WRAPPER-PATCHER.DAT"} {
		if _, err := clientPath(dir, bad); err == nil {
			t.Errorf("%q was allowed", bad)
		}
	}
}

func TestGRFTargets(t *testing.T) {
	for _, target := range []string{"data.grf", "DATA.GRF", `..\x.grf`, "x.txt"} {
		a := &thorArchive{GRFMerging: true, GRF: target}
		if err := applyToGRF(t.TempDir(), a); err == nil {
			t.Errorf("%q was allowed", target)
		}
	}
}

func TestGRFKey(t *testing.T) {
	if grfKey(`Data/Texture\A.BMP`) != `data\texture\a.bmp` {
		t.Error(grfKey(`Data/Texture\A.BMP`))
	}
	// 0xC0 0x41 is one CP949 character whose second byte is 'A'.
	if grfKey("\xc0\x41B") != "\xc0\x41b" {
		t.Errorf("%q", grfKey("\xc0\x41B"))
	}
}

// Dead space from replaced files is dropped once it outgrows live content.
func TestGRFCompacts(t *testing.T) {
	path := filepath.Join(t.TempDir(), "x.grf")
	big := bytes.Repeat([]byte{0}, 3<<20)
	for i := 0; i < 3; i++ {
		g, err := openGRF(path)
		if err != nil {
			t.Fatal(err)
		}
		// Random content, so it doesn't compress away.
		content := make([]byte, len(big))
		rand.New(rand.NewSource(int64(i))).Read(content)
		z := deflate(t, content)
		if err := g.add(`data\big.bin`, bytes.NewReader(z), uint32(len(z)), uint32(len(content))); err != nil {
			t.Fatal(err)
		}
		g.add(`data\small.txt`, bytes.NewReader(deflate(t, []byte("hi"))), uint32(len(deflate(t, []byte("hi")))), 2)
		if err := g.commit(); err != nil {
			t.Fatal(err)
		}
		if g.wasteful() {
			if err := g.compact(path); err != nil {
				t.Fatal(err)
			}
		} else {
			g.close()
		}
	}
	st, _ := os.Stat(path)
	files := grfFiles(t, path)
	if len(files) != 2 || files[`data\small.txt`] != "hi" || len(files[`data\big.bin`]) != len(big) {
		t.Fatalf("after compacting: %d files", len(files))
	}
	if st.Size() > 2*int64(len(deflate(t, []byte(files[`data\big.bin`]))))+4096 {
		t.Errorf("GRF is %d bytes after compacting", st.Size())
	}
	if exists(path + ".tmp") {
		t.Error("the temporary file is left behind")
	}
}
