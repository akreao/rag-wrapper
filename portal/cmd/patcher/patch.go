package main

import (
	"bufio"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"time"
)

// Files the patcher keeps beside itself in the client folder.
const (
	configName   = "rag-wrapper-patcher.json"     // written by the portal into the download
	stateName    = "rag-wrapper-patcher.dat"      // the last patch applied: rpatchur's cache file
	downloadName = "rag-wrapper-patcher.download" // a patch being downloaded
	defaultGRF   = "rag-wrapper.grf"
	clientExe    = "rag-wrapper.exe"
	setupExe     = "opensetup.exe" // the client's settings program, which WARPGATE ships
)

// config is the file the portal puts in the patcher's download.
type config struct {
	Portal string `json:"portal"` // the portal's address, e.g. http://192.168.1.10:8080
}

func loadConfig(dir string) (config, error) {
	var c config
	b, err := os.ReadFile(filepath.Join(dir, configName))
	if err != nil {
		return c, err
	}
	if err := json.Unmarshal(b, &c); err != nil {
		return c, fmt.Errorf("%s: %w", configName, err)
	}
	c.Portal = strings.TrimRight(c.Portal, "/")
	if !strings.HasPrefix(c.Portal, "http://") && !strings.HasPrefix(c.Portal, "https://") {
		return c, fmt.Errorf("%s has no portal address", configName)
	}
	return c, nil
}

// serverInfo is the portal's /patcher/info.json.
type serverInfo struct {
	ServerName string `json:"server_name"`
	News       *struct {
		Title string `json:"title"`
		Text  string `json:"text"`
	} `json:"news"`
	AccountURL string `json:"account_url"`
	ConnectURL string `json:"connect_url"`
	NewsURL    string `json:"news_url"`
	PlistURL   string `json:"plist_url"`
	PatchURL   string `json:"patch_url"` // a patch's name goes on the end
	Patcher    struct {
		URL    string `json:"url"`
		SHA256 string `json:"sha256"`
	} `json:"patcher"`
}

var httpClient = &http.Client{Timeout: 30 * time.Minute}

// get fetches a URL, failing on anything but 200. Connecting gets its own
// short timeout through ctx.
func get(ctx context.Context, url string) (*http.Response, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, url, nil)
	if err != nil {
		return nil, err
	}
	req.Header.Set("Cache-Control", "no-cache")
	resp, err := httpClient.Do(req)
	if err != nil {
		return nil, err
	}
	if resp.StatusCode != http.StatusOK {
		resp.Body.Close()
		return nil, fmt.Errorf("%s: %s", url, resp.Status)
	}
	return resp, nil
}

func fetchInfo(ctx context.Context, c config) (*serverInfo, error) {
	ctx, cancel := context.WithTimeout(ctx, 15*time.Second)
	defer cancel()
	resp, err := get(ctx, c.Portal+"/patcher/info.json")
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()
	var info serverInfo
	if err := json.NewDecoder(io.LimitReader(resp.Body, 1<<20)).Decode(&info); err != nil {
		return nil, fmt.Errorf("the server's patcher info: %w", err)
	}
	if info.PlistURL == "" || info.PatchURL == "" {
		return nil, errors.New("the server's patcher info has no patch list")
	}
	return &info, nil
}

// plistEntry is one line of plist.txt: "<number> <file>".
type plistEntry struct {
	N    int
	Name string
}

func parsePlist(r io.Reader) ([]plistEntry, error) {
	var list []plistEntry
	sc := bufio.NewScanner(r)
	for sc.Scan() {
		line := strings.TrimSpace(sc.Text())
		if line == "" || strings.HasPrefix(line, "//") {
			continue
		}
		fields := strings.Fields(line)
		if len(fields) < 2 {
			continue
		}
		n, err := strconv.Atoi(fields[0])
		if err != nil || !safeName(fields[1]) {
			continue
		}
		list = append(list, plistEntry{n, fields[1]})
	}
	return list, sc.Err()
}

// safeName is a plain file name that can go on a URL or in the client
// folder as it is: the names the portal serves.
func safeName(s string) bool {
	if s == "" || s[0] == '.' || len(s) > 200 {
		return false
	}
	for _, c := range s {
		if !(c >= 'a' && c <= 'z' || c >= 'A' && c <= 'Z' || c >= '0' && c <= '9' || c == '.' || c == '_' || c == '-') {
			return false
		}
	}
	return true
}

// state is rpatchur's cache file, in its format and under its name, so a
// player who switches between rpatchur and this patcher (either way) keeps
// their place in the patch list.
type state struct {
	Applied int `json:"last_patch_index"` // the plist number of the last patch applied
}

func loadState(dir string) state {
	var s state
	if b, err := os.ReadFile(filepath.Join(dir, stateName)); err == nil {
		json.Unmarshal(b, &s)
	}
	return s
}

func saveState(dir string, s state) error {
	b, _ := json.Marshal(s)
	return writeFileAtomic(filepath.Join(dir, stateName), b)
}

// pending is what's left to apply. Like rpatchur, a last-applied number
// that isn't in the list means the list was started over (the server's
// patches/ folder emptied), so everything applies again.
func pending(list []plistEntry, applied int) []plistEntry {
	known := false
	for _, p := range list {
		known = known || p.N == applied
	}
	var out []plistEntry
	for _, p := range list {
		if !known || p.N > applied {
			out = append(out, p)
		}
	}
	return out
}

// progress reports what the patcher is doing. fraction is from 0 to 1 for
// the whole update.
type progress func(text string, fraction float64)

// update brings the client folder up to date with the server's patches.
func update(ctx context.Context, dir string, info *serverInfo, report progress) error {
	report("Checking for updates...", 0)
	resp, err := get(ctx, info.PlistURL)
	if err != nil {
		return err
	}
	list, err := parsePlist(io.LimitReader(resp.Body, 1<<20))
	resp.Body.Close()
	if err != nil {
		return err
	}
	st := loadState(dir)
	todo := pending(list, st.Applied)
	for i, p := range todo {
		share := 1 / float64(len(todo))
		base := float64(i) * share
		label := fmt.Sprintf("patch %d of %d", i+1, len(todo))
		path := filepath.Join(dir, downloadName)
		err := download(ctx, info.PatchURL+p.Name, path, func(done, total int64) {
			f := 0.0
			if total > 0 {
				f = float64(done) / float64(total)
			}
			report(fmt.Sprintf("Downloading %s (%s, %s)", label, p.Name, sizeText(done, total)), base+share*0.8*f)
		})
		if err != nil {
			return fmt.Errorf("downloading %s: %w", p.Name, err)
		}
		report(fmt.Sprintf("Applying %s (%s)", label, p.Name), base+share*0.8)
		err = applyFile(dir, path)
		os.Remove(path)
		if err != nil {
			return fmt.Errorf("applying %s: %w", p.Name, err)
		}
		st.Applied = p.N
		if err := saveState(dir, st); err != nil {
			return err
		}
	}
	report("Ready to play.", 1)
	return nil
}

func sizeText(done, total int64) string {
	mb := func(n int64) string { return strconv.FormatFloat(float64(n)/(1<<20), 'f', 1, 64) }
	if total > 0 {
		return mb(done) + " of " + mb(total) + " MB"
	}
	return mb(done) + " MB"
}

func download(ctx context.Context, url, path string, report func(done, total int64)) error {
	resp, err := get(ctx, url)
	if err != nil {
		return err
	}
	defer resp.Body.Close()
	f, err := os.Create(path)
	if err != nil {
		return err
	}
	defer f.Close()
	var done int64
	buf := make([]byte, 256<<10)
	last := time.Time{}
	for {
		n, rerr := resp.Body.Read(buf)
		if n > 0 {
			if _, err := f.Write(buf[:n]); err != nil {
				return err
			}
			done += int64(n)
			if time.Since(last) > 100*time.Millisecond {
				report(done, resp.ContentLength)
				last = time.Now()
			}
		}
		if rerr == io.EOF {
			break
		}
		if rerr != nil {
			return rerr
		}
	}
	if resp.ContentLength > 0 && done != resp.ContentLength {
		return fmt.Errorf("got %d of %d bytes", done, resp.ContentLength)
	}
	report(done, resp.ContentLength)
	return f.Close()
}

// applyFile applies one downloaded THOR patch to the client folder.
func applyFile(dir, path string) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	st, err := f.Stat()
	if err != nil {
		return err
	}
	a, err := readThor(f, st.Size())
	if err != nil {
		return err
	}
	if err := a.checkIntegrity(); err != nil {
		return err
	}
	if a.GRFMerging {
		return applyToGRF(dir, a)
	}
	return applyToFolder(dir, a)
}

func applyToGRF(dir string, a *thorArchive) error {
	name := a.GRF
	if name == "" {
		name = defaultGRF
	}
	if !safeName(name) || !strings.HasSuffix(strings.ToLower(name), ".grf") {
		return fmt.Errorf("the patch targets %q, which isn't a GRF in the client folder", name)
	}
	if strings.EqualFold(name, "data.grf") {
		return errors.New("the patch targets data.grf, the official client's GRF, which the patcher never changes")
	}
	path := filepath.Join(dir, name)
	g, err := openGRF(path)
	if err != nil {
		return inUse(err)
	}
	for _, e := range a.Entries {
		if strings.EqualFold(e.Path, integrityFile) {
			continue
		}
		if e.Remove {
			g.remove(e.Path)
			continue
		}
		if err := g.add(e.Path, a.raw(e), e.Compressed, e.Size); err != nil {
			g.close()
			return err
		}
	}
	if err := g.commit(); err != nil {
		g.close()
		return inUse(err)
	}
	if g.wasteful() {
		return inUse(g.compact(path))
	}
	return g.close()
}

func applyToFolder(dir string, a *thorArchive) error {
	for _, e := range a.Entries {
		if strings.EqualFold(e.Path, integrityFile) {
			continue
		}
		path, err := clientPath(dir, e.Path)
		if err != nil {
			return err
		}
		if e.Remove {
			if err := os.Remove(path); err != nil && !errors.Is(err, os.ErrNotExist) {
				return inUse(err)
			}
			continue
		}
		b, err := a.content(e)
		if err != nil {
			return err
		}
		if err := os.MkdirAll(filepath.Dir(path), 0o755); err != nil {
			return err
		}
		if err := writeFileAtomic(path, b); err != nil {
			return inUse(err)
		}
	}
	return nil
}

// clientPath is where a patch path goes in the client folder. Patches only
// write inside it, never over the patcher's own files.
func clientPath(dir, p string) (string, error) {
	rel := strings.ReplaceAll(p, `\`, "/")
	bad := rel == "" || strings.HasPrefix(rel, "/") || strings.Contains(rel, ":")
	for _, part := range strings.Split(rel, "/") {
		bad = bad || part == "" || part == "." || part == ".."
	}
	for i := 0; i < len(rel); i++ {
		bad = bad || rel[i] < 0x20 || rel[i] >= 0x80
	}
	if bad {
		return "", fmt.Errorf("the patch has a file name the patcher won't write: %q", p)
	}
	for _, own := range []string{configName, stateName, downloadName} {
		if strings.EqualFold(rel, own) {
			return "", fmt.Errorf("the patch would replace the patcher's own %s", own)
		}
	}
	if exe, err := os.Executable(); err == nil && strings.EqualFold(filepath.Clean(filepath.Join(dir, rel)), filepath.Clean(exe)) {
		return "", errors.New("the patch would replace the patcher itself")
	}
	return filepath.Join(dir, filepath.FromSlash(rel)), nil
}

// writeFileAtomic writes beside the file, then swaps it in, so a half
// written file is never left in its place.
func writeFileAtomic(path string, b []byte) error {
	tmp := path + ".rwp-tmp"
	if err := os.WriteFile(tmp, b, 0o644); err != nil {
		return err
	}
	if err := os.Rename(tmp, path); err != nil {
		os.Remove(tmp)
		return err
	}
	return nil
}

// inUse explains the usual reason Windows refuses to replace a client
// file: the game is still open.
func inUse(err error) error {
	if err == nil || !isSharingViolation(err) {
		return err
	}
	return fmt.Errorf("%w. Close the game and try again", err)
}
