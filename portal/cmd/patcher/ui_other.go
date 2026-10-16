//go:build !windows

package main

import (
	"context"
	"flag"
	"fmt"
	"os"
	"strings"
)

// run patches a client folder from a terminal: the same update as the
// Windows window, without Play or self-update (the portal's patcher is a
// Windows program).
func run() int {
	dir := flag.String("dir", "", "the client folder (default: the patcher's own folder)")
	flag.Parse()
	if *dir == "" {
		d, err := clientDir()
		if err != nil {
			fmt.Fprintln(os.Stderr, err)
			return 1
		}
		*dir = d
	}
	cfg, err := loadConfig(*dir)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 1
	}
	ctx := context.Background()
	info, err := fetchInfo(ctx, *dir, &cfg)
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 1
	}
	fmt.Printf("%s (%s)\n", info.ServerName, cfg.Portal)
	if info.News != nil {
		fmt.Printf("\n%s\n%s\n\n", info.News.Title, info.News.Text)
	}
	last := ""
	err = update(ctx, *dir, info, func(text string, fraction float64) {
		// One line per step and per tenth of the way, not per chunk.
		step, _, _ := strings.Cut(text, " (")
		if key := fmt.Sprintf("%s %d", step, int(fraction*10)); key != last {
			fmt.Printf("%3.0f%% %s\n", fraction*100, text)
			last = key
		}
	})
	if err != nil {
		fmt.Fprintln(os.Stderr, err)
		return 1
	}
	return 0
}
