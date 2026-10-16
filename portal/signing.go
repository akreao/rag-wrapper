package main

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"io"
	"os"
	"path/filepath"
	"sync"
	"time"
)

// Our own patcher (cmd/patcher) only applies patches and updates itself
// from what the portal has signed, so someone in the middle of the LAN or
// VPN can't hand players a program of their own. The portal signs
// info.json, which lists each published patch's SHA-256 and the patcher
// exe's, with an ed25519 key made from PATCH_SIGNING_KEY, a secret the
// desktop app keeps in .env. The patcher download carries the public key,
// and a patcher from before signing learns it from the first info.json it
// gets. rpatchur reads plist.txt and the patches as before; it has no way
// to check a signature.

// signingContext starts every signed message, so a signature made here
// can't be passed off as one over anything else. Shared with cmd/patcher.
const signingContext = "rag-wrapper patcher info v1\n"

// signingKey is the key PATCH_SIGNING_KEY makes, or nil when it's unset and
// nothing is signed. Any string works; the desktop app uses 40 random
// letters and digits.
func signingKey(secret string) ed25519.PrivateKey {
	if secret == "" {
		return nil
	}
	seed := sha256.Sum256([]byte("rag-wrapper patch signing key\n" + secret))
	return ed25519.NewKeyFromSeed(seed[:])
}

// publicKey is the key the patcher checks signatures with, as it goes in
// its config and info.json.
func publicKey(key ed25519.PrivateKey) string {
	return base64.StdEncoding.EncodeToString(key.Public().(ed25519.PublicKey))
}

// signInfo adds the signature to info.json: the whole info, encoded once,
// goes in "signed" with its signature beside it. The plain fields stay for
// patchers from before signing, which read them to update themselves.
func signInfo(key ed25519.PrivateKey, info map[string]any) error {
	payload, err := json.Marshal(info)
	if err != nil {
		return err
	}
	info["public_key"] = publicKey(key)
	info["signed"] = base64.StdEncoding.EncodeToString(payload)
	info["signature"] = base64.StdEncoding.EncodeToString(ed25519.Sign(key, append([]byte(signingContext), payload...)))
	return nil
}

// signedPatch is one patch in the signed info, in plist.txt order. A patch
// that can't be read has no checksum, and the patcher stops at it.
type signedPatch struct {
	N      int    `json:"n"`
	Name   string `json:"name"`
	Size   int64  `json:"size"`
	SHA256 string `json:"sha256"`
}

func (p patcherFiles) signedPatches() []signedPatch {
	list := []signedPatch{}
	for _, l := range p.listed() {
		s := signedPatch{N: l.n, Name: l.name}
		s.Size, s.SHA256 = fileSum(filepath.Join(p.Dir, l.name))
		list = append(list, s)
	}
	return list
}

// File checksums, kept until a file changes: patches can be hundreds of MB,
// and every patcher start asks for info.json.
var sums = struct {
	sync.Mutex
	m map[string]fileSumEntry
}{m: map[string]fileSumEntry{}}

type fileSumEntry struct {
	mod  time.Time
	size int64
	sum  string
}

// fileSum is a regular file's size and SHA-256, or 0 and "" when it can't
// be read.
func fileSum(path string) (int64, string) {
	st, err := os.Stat(path)
	if err != nil || !st.Mode().IsRegular() {
		return 0, ""
	}
	sums.Lock()
	defer sums.Unlock()
	if c, ok := sums.m[path]; ok && c.mod.Equal(st.ModTime()) && c.size == st.Size() {
		return c.size, c.sum
	}
	f, err := os.Open(path)
	if err != nil {
		return 0, ""
	}
	defer f.Close()
	h := sha256.New()
	n, err := io.Copy(h, f)
	if err != nil || n != st.Size() {
		return 0, ""
	}
	sums.m[path] = fileSumEntry{st.ModTime(), n, hex.EncodeToString(h.Sum(nil))}
	return n, sums.m[path].sum
}
