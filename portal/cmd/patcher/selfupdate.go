package main

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"io"
	"os"
	"strings"
	"time"
)

// The patcher updates itself: when the portal's copy differs from the
// running one, it downloads it, swaps it in and starts it. Windows lets a
// running program's file be renamed but not replaced, so the old one moves
// aside to <name>.old, which the new one deletes once the old has exited.

// updatedFlag tells a freshly swapped-in patcher not to check again, so a
// mismatch the patcher can't resolve never loops.
const updatedFlag = "-updated"

func fileSHA256(path string) (string, error) {
	f, err := os.Open(path)
	if err != nil {
		return "", err
	}
	defer f.Close()
	h := sha256.New()
	if _, err := io.Copy(h, f); err != nil {
		return "", err
	}
	return hex.EncodeToString(h.Sum(nil)), nil
}

// needsUpdate is true when the portal offers a patcher that isn't this one.
func needsUpdate(info *serverInfo, exe string) bool {
	want := strings.ToLower(info.Patcher.SHA256)
	if want == "" || info.Patcher.URL == "" {
		return false
	}
	have, err := fileSHA256(exe)
	return err == nil && have != want
}

// replaceSelf downloads the portal's patcher and puts it where exe is,
// keeping the running one as exe+".old". On failure exe is left as it was.
func replaceSelf(ctx context.Context, info *serverInfo, exe string) error {
	next := exe + ".new"
	defer os.Remove(next)
	if err := download(ctx, info.Patcher.URL, next, func(int64, int64) {}); err != nil {
		return err
	}
	got, err := fileSHA256(next)
	if err != nil {
		return err
	}
	if got != strings.ToLower(info.Patcher.SHA256) {
		return fmt.Errorf("the downloaded patcher's checksum is %s, expected %s", got, info.Patcher.SHA256)
	}
	old := exe + ".old"
	os.Remove(old)
	if err := os.Rename(exe, old); err != nil {
		return err
	}
	if err := os.Rename(next, exe); err != nil {
		os.Rename(old, exe)
		return err
	}
	return nil
}

// removeOld deletes the previous patcher left by an update, giving it a
// moment to exit.
func removeOld(exe string) {
	old := exe + ".old"
	for i := 0; i < 20; i++ {
		err := os.Remove(old)
		if err == nil || os.IsNotExist(err) {
			return
		}
		time.Sleep(250 * time.Millisecond)
	}
}
