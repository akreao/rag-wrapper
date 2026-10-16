package main

import (
	"crypto/ed25519"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"net/url"
	"os"
	"path/filepath"
	"strings"
)

// The portal signs info.json (portal/signing.go), which lists each patch's
// SHA-256 and the patcher exe's, so the patcher applies only what the
// server published, even when someone in the middle of the LAN or VPN
// changes what it downloads. The public key comes in the patcher's
// download; a patcher set up before signing takes it from the first
// info.json it gets and keeps it in its config from then on.

// signingContext starts every signed message. Shared with the portal.
const signingContext = "rag-wrapper patcher info v1\n"

// signedPatch is one patch in the signed info, in plist.txt order.
type signedPatch struct {
	N      int    `json:"n"`
	Name   string `json:"name"`
	Size   int64  `json:"size"`
	SHA256 string `json:"sha256"`
}

// trustError is a signature problem, which the window shows as it is
// rather than as a connection problem.
type trustError struct{ msg string }

func (e *trustError) Error() string { return e.msg }

const redownload = " Download the patcher again from the server's \"How to connect\" page if the server was set up again; otherwise someone on the network may be changing the patcher's downloads."

// trust checks the info's signature against the key in the config, taking
// the server's key when the config has none yet, and returns the signed
// info. A server that has never signed anything is trusted as before.
func trust(dir string, c *config, raw *serverInfo) (*serverInfo, error) {
	key := c.PublicKey
	if key == "" {
		if raw.PublicKey == "" && raw.Signed == "" {
			return raw, nil
		}
		key = raw.PublicKey
	} else if raw.PublicKey != "" && raw.PublicKey != key {
		return nil, &trustError{"The server's signing key has changed, so the patcher can't tell whether its updates come from the server." + redownload}
	}
	info, err := verify(key, raw)
	if err != nil {
		return nil, err
	}
	if c.PublicKey == "" {
		// Keeping the key can fail in a read-only folder; it's taken again
		// next time.
		c.PublicKey = key
		saveKey(dir, key)
	}
	return info, nil
}

func verify(key string, raw *serverInfo) (*serverInfo, error) {
	pub, err := base64.StdEncoding.DecodeString(key)
	if err != nil || len(pub) != ed25519.PublicKeySize {
		return nil, &trustError{"The patcher's signing key in " + configName + " is damaged." + redownload}
	}
	if raw.Signed == "" || raw.Signature == "" {
		return nil, &trustError{"The server's update list isn't signed, so the patcher won't use it." + redownload}
	}
	payload, err1 := base64.StdEncoding.DecodeString(raw.Signed)
	sig, err2 := base64.StdEncoding.DecodeString(raw.Signature)
	if err1 != nil || err2 != nil || !ed25519.Verify(pub, append([]byte(signingContext), payload...), sig) {
		return nil, &trustError{"The server's update list doesn't match its signature, so the patcher won't use it." + redownload}
	}
	var info serverInfo
	if err := json.Unmarshal(payload, &info); err != nil {
		return nil, fmt.Errorf("the server's signed patcher info: %w", err)
	}
	if info.PatchURL == "" || info.Patches == nil {
		return nil, errors.New("the server's signed patcher info has no patch list")
	}
	info.signed = true
	return &info, nil
}

// saveKey adds the key to the patcher's config, keeping what else is in it.
func saveKey(dir, key string) error {
	path := filepath.Join(dir, configName)
	b, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var m map[string]any
	if err := json.Unmarshal(b, &m); err != nil {
		return err
	}
	m["public_key"] = key
	b, err = json.MarshalIndent(m, "", "  ")
	if err != nil {
		return err
	}
	return writeFileAtomic(path, append(b, '\n'))
}

// checkDownload compares a downloaded patch with its signed size and
// checksum.
func checkDownload(path string, p signedPatch) error {
	f, err := os.Open(path)
	if err != nil {
		return err
	}
	defer f.Close()
	h := sha256.New()
	n, err := io.Copy(h, f)
	if err != nil {
		return err
	}
	if got := hex.EncodeToString(h.Sum(nil)); n != p.Size || p.SHA256 == "" || !strings.EqualFold(got, p.SHA256) {
		return &trustError{p.Name + " isn't the patch the server signed, so the patcher won't apply it. Try again; if it keeps happening, someone on the network may be changing the patcher's downloads."}
	}
	return nil
}

// safeLink is u when it's a web page on the portal the patcher talks to,
// and "" otherwise, so the link buttons never open a program or a file.
func safeLink(portal, u string) string {
	p, err1 := url.Parse(portal)
	l, err2 := url.Parse(u)
	if err1 != nil || err2 != nil || (l.Scheme != "http" && l.Scheme != "https") || l.User != nil || !strings.EqualFold(l.Host, p.Host) {
		return ""
	}
	return l.String()
}
