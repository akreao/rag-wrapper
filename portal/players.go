package main

import (
	"bytes"
	"context"
	"html/template"
	"io"
	"log"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"strconv"
	"strings"

	"github.com/yuin/goldmark"
	"github.com/yuin/goldmark/extension"
)

// Public pages for players: who's online, rankings, and the server's news
// and rules. None of them need an account.

// PlayerCharacter is a character as the public pages show it.
type PlayerCharacter struct {
	Name      string
	Class     int
	BaseLevel int
	JobLevel  int
	Map       string
	Zeny      int64
}

func (c PlayerCharacter) JobName() string { return jobName(c.Class) }

// onlineCharacters lists the characters the char server marks online, the
// same list the desktop app's dashboard shows.
func (s *store) onlineCharacters(ctx context.Context) ([]PlayerCharacter, error) {
	rows, err := s.db.QueryContext(ctx,
		"SELECT `name`, `class`, `base_level`, `job_level`, `last_map`, 0 FROM "+s.chars+"`char` WHERE `online` = 1 ORDER BY `name`")
	if err != nil {
		return nil, err
	}
	return scanPlayerCharacters(rows)
}

// Rankings: what characters can be ranked by, as the ?by= value.
const (
	rankByLevel = "level"
	rankByZeny  = "zeny"
)

// rankingLimit is how many characters a ranking lists.
const rankingLimit = 50

// rankedFrom is the characters that may appear in rankings: those of
// player accounts that aren't admins (whose GM characters would top every
// list) and aren't blocked.
func (s *store) rankedFrom() string {
	return " FROM " + s.chars + "`char` c JOIN `login` l ON l.`account_id` = c.`account_id`" +
		" WHERE l.`sex` <> 'S' AND l.`group_id` <> ? AND l.`state` = 0"
}

// rankings lists the top characters by base level (then experience) or by
// zeny. class < 0 ranks every class.
func (s *store) rankings(ctx context.Context, by string, class, adminGroup int) ([]PlayerCharacter, error) {
	query := "SELECT c.`name`, c.`class`, c.`base_level`, c.`job_level`, '', c.`zeny`" + s.rankedFrom()
	args := []any{adminGroup}
	if class >= 0 {
		query += " AND c.`class` = ?"
		args = append(args, class)
	}
	if by == rankByZeny {
		query += " ORDER BY c.`zeny` DESC, c.`base_level` DESC"
	} else {
		query += " ORDER BY c.`base_level` DESC, c.`base_exp` DESC, c.`job_level` DESC, c.`job_exp` DESC"
	}
	query += ", c.`char_id` LIMIT ?"
	rows, err := s.db.QueryContext(ctx, query, append(args, rankingLimit)...)
	if err != nil {
		return nil, err
	}
	return scanPlayerCharacters(rows)
}

// rankedClasses lists the classes of the characters rankings can show,
// for the class filter.
func (s *store) rankedClasses(ctx context.Context, adminGroup int) ([]int, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT DISTINCT c.`class`"+s.rankedFrom()+" ORDER BY c.`class`", adminGroup)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var classes []int
	for rows.Next() {
		var c int
		if err := rows.Scan(&c); err != nil {
			return nil, err
		}
		classes = append(classes, c)
	}
	return classes, rows.Err()
}

func scanPlayerCharacters(rows interface {
	Next() bool
	Scan(...any) error
	Err() error
	Close() error
}) ([]PlayerCharacter, error) {
	defer rows.Close()
	var chars []PlayerCharacter
	for rows.Next() {
		var c PlayerCharacter
		if err := rows.Scan(&c.Name, &c.Class, &c.BaseLevel, &c.JobLevel, &c.Map, &c.Zeny); err != nil {
			return nil, err
		}
		chars = append(chars, c)
	}
	return chars, rows.Err()
}

// --- Who's online ---

func (a *app) online(req *request) error {
	chars, err := a.store.onlineCharacters(req.r.Context())
	if err != nil {
		return err
	}
	a.render(req, http.StatusOK, "online", view{Title: "Who's online", Data: chars})
	return nil
}

// --- Rankings ---

type rankingsData struct {
	By      string
	Class   int // -1 for every class
	Classes []int
	Chars   []PlayerCharacter
}

// URL links to a ranking, keeping the class filter.
func (d rankingsData) URL(by string) string {
	q := url.Values{}
	if by != rankByLevel {
		q.Set("by", by)
	}
	if d.Class >= 0 {
		q.Set("class", strconv.Itoa(d.Class))
	}
	if len(q) == 0 {
		return "/rankings"
	}
	return "/rankings?" + q.Encode()
}

func (a *app) rankings(req *request) error {
	q := req.r.URL.Query()
	d := rankingsData{By: rankByLevel, Class: -1}
	if q.Get("by") == rankByZeny {
		d.By = rankByZeny
	}
	if c, err := strconv.Atoi(q.Get("class")); err == nil && c >= 0 {
		d.Class = c
	}
	ctx := req.r.Context()
	var err error
	if d.Classes, err = a.store.rankedClasses(ctx, a.adminGroup); err != nil {
		return err
	}
	if d.Chars, err = a.store.rankings(ctx, d.By, d.Class, a.adminGroup); err != nil {
		return err
	}
	a.render(req, http.StatusOK, "rankings", view{Title: "Rankings", Data: d})
	return nil
}

// --- News and rules ---

// newsFile is the news page's Markdown, in the stack's overrides folder
// (mounted read-only). It's read on every request, so an edit shows
// straight away.
const newsFile = "portal/news.md"

// newsMaxSize keeps a stray huge file from being rendered on every request.
const newsMaxSize = 256 << 10

// The renderer leaves out raw HTML and javascript: links (goldmark's
// default), so the file can't put scripts on the portal.
var markdown = goldmark.New(goldmark.WithExtensions(extension.GFM))

type newsData struct {
	Body    template.HTML // the whole file
	Latest  template.HTML // the first "## " section: the newest post
	Missing bool          // no news file yet
	Titled  bool          // the file starts with its own "# " title
}

// news reads and renders the news file. Posts are "## " sections, newest
// first; anything above the first one (such as the rules) shows only on
// the news page.
func (c connectInfo) news() newsData {
	f, err := os.Open(filepath.Join(c.Overrides, filepath.FromSlash(newsFile)))
	if err != nil {
		return newsData{Missing: true}
	}
	defer f.Close()
	src, err := io.ReadAll(io.LimitReader(f, newsMaxSize))
	if err != nil {
		log.Printf("news: %v", err)
		return newsData{Missing: true}
	}
	if len(bytes.TrimSpace(src)) == 0 {
		return newsData{Missing: true}
	}
	return newsData{
		Body:   renderMarkdown(src),
		Latest: renderMarkdown(latestPost(src)),
		Titled: strings.HasPrefix(string(bytes.TrimSpace(src)), "# "),
	}
}

// latestPost is the first "## " section of the news file, up to the next
// heading of level 1 or 2. Lines inside ``` or ~~~ code blocks don't count.
func latestPost(src []byte) []byte {
	var post []string
	fence := ""
	for _, line := range strings.SplitAfter(string(src), "\n") {
		trimmed := strings.TrimSpace(line)
		if fence == "" && (strings.HasPrefix(trimmed, "# ") || strings.HasPrefix(trimmed, "## ") || trimmed == "#" || trimmed == "##") {
			if post != nil {
				break
			}
			if strings.HasPrefix(trimmed, "## ") || trimmed == "##" {
				post = []string{}
			}
		}
		if post != nil {
			post = append(post, line)
		}
		if f := fenceOf(trimmed); f != "" && (fence == "" || f == fence) {
			if fence == "" {
				fence = f
			} else {
				fence = ""
			}
		}
	}
	return []byte(strings.Join(post, ""))
}

func fenceOf(line string) string {
	for _, f := range []string{"```", "~~~"} {
		if strings.HasPrefix(line, f) {
			return f
		}
	}
	return ""
}

func renderMarkdown(src []byte) template.HTML {
	if len(bytes.TrimSpace(src)) == 0 {
		return ""
	}
	var buf bytes.Buffer
	if err := markdown.Convert(src, &buf); err != nil {
		log.Printf("news: %v", err)
		return ""
	}
	return template.HTML(buf.String()) // goldmark escapes the text and drops raw HTML
}

func (a *app) newsPage(req *request) error {
	a.render(req, http.StatusOK, "news", view{Title: "News", Data: a.conn.news()})
	return nil
}

// thousands writes n with commas between groups of three digits.
func thousands(n int64) string {
	s := strconv.FormatInt(n, 10)
	neg := strings.HasPrefix(s, "-")
	s = strings.TrimPrefix(s, "-")
	var b strings.Builder
	for i, d := range s {
		if i > 0 && (len(s)-i)%3 == 0 {
			b.WriteByte(',')
		}
		b.WriteRune(d)
	}
	if neg {
		return "-" + b.String()
	}
	return b.String()
}
