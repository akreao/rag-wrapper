// Command portal is the account web portal for the rag-wrapper stack.
//
// It serves plain HTML pages for registering and managing rAthena accounts,
// talking directly to the same MariaDB database the login server uses.
package main

import (
	"context"
	"database/sql"
	"embed"
	"errors"
	"log"
	"net"
	"net/http"
	"os"
	"os/signal"
	"strconv"
	"strings"
	"syscall"
	"time"

	"github.com/go-sql-driver/mysql"
)

//go:embed templates/*.html static/*
var assets embed.FS

func main() {
	cfg := mysql.NewConfig()
	cfg.Net = "tcp"
	cfg.Addr = net.JoinHostPort(env("DB_HOST", "db"), env("DB_PORT", "3306"))
	cfg.DBName = env("DB_NAME", "ragnarok")
	cfg.User = env("DB_USER", "ragnarok")
	cfg.Passwd = os.Getenv("DB_PASSWORD")
	cfg.Timeout = 5 * time.Second
	if cfg.Passwd == "" {
		log.Fatal("DB_PASSWORD must be set")
	}

	db, err := sql.Open("mysql", cfg.FormatDSN())
	if err != nil {
		log.Fatal(err)
	}
	db.SetMaxOpenConns(10)
	db.SetConnMaxLifetime(5 * time.Minute)
	waitForDB(db)

	adminGroup := 99
	if v := os.Getenv("ADMIN_GROUP"); v != "" {
		if adminGroup, err = strconv.Atoi(v); err != nil {
			log.Fatalf("ADMIN_GROUP: %v", err)
		}
	}

	conn := connectInfo{
		Address:   env("PUBLIC_IP", "127.0.0.1"),
		PacketVer: env("PACKETVER", "20250716"),
		Renewal:   env("PRERE", "yes") == "no",
		Overrides: env("OVERRIDES_DIR", "/overrides"),
		Patcher: patcherFiles{
			Dir: env("PATCHES_DIR", "/patches"),
			Exe: env("PATCHER_EXE", "/patcher/rpatchur.exe"),
		},
	}

	app, err := newApp(&store{db: db, chars: charsPrefix(cfg.DBName, conn.Renewal)}, adminGroup, conn)
	if err != nil {
		log.Fatal(err)
	}

	srv := &http.Server{
		Addr:              env("PORTAL_ADDR", ":8080"),
		Handler:           app.routes(),
		ReadHeaderTimeout: 10 * time.Second,
		ReadTimeout:       30 * time.Second,
		WriteTimeout:      30 * time.Second,
		IdleTimeout:       120 * time.Second,
	}

	go func() {
		stop := make(chan os.Signal, 1)
		signal.Notify(stop, syscall.SIGINT, syscall.SIGTERM)
		<-stop
		ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
		defer cancel()
		srv.Shutdown(ctx)
	}()

	log.Printf("portal listening on %s", srv.Addr)
	if err := srv.ListenAndServe(); err != nil && !errors.Is(err, http.ErrServerClosed) {
		log.Fatal(err)
	}
}

// charsPrefix names the game mode's characters database, which db-init
// makes next to the accounts database, as a prefix for its tables.
func charsPrefix(accounts string, renewal bool) string {
	name := env("CHAR_DB_NAME", accounts+"_prere")
	if renewal {
		name = env("CHAR_DB_NAME", accounts+"_re")
	}
	return "`" + strings.ReplaceAll(name, "`", "``") + "`."
}

// waitForDB gives the database a minute to come up, so the portal can
// start alongside it. It keeps going if the database never answers; pages
// then show an error until it does.
func waitForDB(db *sql.DB) {
	deadline := time.Now().Add(time.Minute)
	for {
		err := db.Ping()
		if err == nil {
			return
		}
		if time.Now().After(deadline) {
			log.Printf("database not reachable yet: %v", err)
			return
		}
		time.Sleep(2 * time.Second)
	}
}

func env(key, def string) string {
	if v := os.Getenv(key); v != "" {
		return v
	}
	return def
}
