// Command patcher is the players' client patcher, a Windows program the
// portal builds and hands out (roadmap feature 17, replacing rpatchur).
//
// It sits in the game client's folder with the config the portal wrote
// into its download, fetches the server's patch list, applies each THOR
// patch it hasn't applied yet (into rag-wrapper.grf or the client folder),
// and starts the patched client when the player clicks Play. It shows the
// server's newest news post meanwhile, and updates itself when the portal
// has a newer build.
//
// On Windows it's a window (ui_windows.go). Elsewhere it runs in a
// terminal, for testing against a portal from Linux.
package main

import (
	"os"
	"path/filepath"
)

func main() {
	os.Exit(run())
}

// clientDir is the folder the patcher is in: the game client's.
func clientDir() (string, error) {
	exe, err := os.Executable()
	if err != nil {
		return "", err
	}
	if exe, err = filepath.EvalSymlinks(exe); err != nil {
		return "", err
	}
	return filepath.Dir(exe), nil
}

// looksLikeClient is true when dir has the official client's data.grf.
func looksLikeClient(dir string) bool {
	st, err := os.Stat(filepath.Join(dir, "data.grf"))
	return err == nil && st.Mode().IsRegular()
}
