package main

import (
	"context"
	"database/sql"
	"errors"
	"net/http"
	"net/url"
	"strconv"
	"strings"
	"time"

	"github.com/go-sql-driver/mysql"
)

// Public item and monster lookup: what an item is and who drops it, and a
// monster's stats, drops and where it spawns. The rAthena image's
// lookup_db.py fills the wrapper_items, wrapper_mobs, wrapper_mob_drops,
// wrapper_mob_spawns and wrapper_lookup_info tables in each game mode's
// characters database whenever the map server starts, from the same files
// the map server loads, so they follow the server's mode, its mods and
// custom items, and its drop and exp rates.

// lookupPageSize is how many items or monsters a search page lists.
const lookupPageSize = 50

// errNoLookup is returned before the map server has filled the tables.
var errNoLookup = errors.New("lookup tables not filled yet")

func noSuchTable(err error) bool {
	var me *mysql.MySQLError
	return errors.As(err, &me) && me.Number == 1146 // ER_NO_SUCH_TABLE
}

// LookupItem is a row of wrapper_items.
type LookupItem struct {
	ID            int
	Name          string
	Aegis         string
	Type          string
	SubType       string
	Buy           int64
	Sell          int64
	Weight        int // in tenths, as rAthena stores it
	Attack        int
	MagicAttack   int
	Defense       int
	Range         int
	Slots         int
	WeaponLevel   int
	ArmorLevel    int
	EquipLevelMin int
	EquipLevelMax int
	Refineable    bool
	Gender        string
	Jobs          string
	Locations     string
	Script        string
	EquipScript   string
	UnEquipScript string
	Source        string // the mod that added it, "import", or "" for rAthena's own
	DroppedBy     int    // how many monsters drop it, on search pages
}

// WeightText is the weight as the game shows it.
func (i LookupItem) WeightText() string { return tenths(i.Weight) }

// Equipment reports whether the item is worn, so its equip stats apply.
func (i LookupItem) Equipment() bool {
	switch i.Type {
	case "Weapon", "Armor", "ShadowGear", "PetArmor":
		return true
	}
	return false
}

// TypeText is the item's type with its subtype, e.g. "Weapon (Dagger)".
func (i LookupItem) TypeText() string {
	if i.SubType != "" {
		return i.Type + " (" + i.SubType + ")"
	}
	return i.Type
}

// Custom names where a custom item came from, or "" for rAthena's items.
func (i LookupItem) Custom() string { return customSource(i.Source) }

func customSource(source string) string {
	switch source {
	case "":
		return ""
	case "import":
		return "Custom"
	}
	return "Mod: " + source
}

// LookupMob is a row of wrapper_mobs. Exp is with the server's rates.
type LookupMob struct {
	ID              int
	Name            string
	Aegis           string
	Level           int
	HP              int64
	SP              int64
	BaseExp         int64
	JobExp          int64
	MvpExp          int64
	Attack          int
	Attack2         int
	Defense         int
	MagicDefense    int
	Resistance      int
	MagicResistance int
	Str, Agi, Vit   int
	Int, Dex, Luk   int
	AttackRange     int
	Size            string
	Race            string
	Element         string
	ElementLevel    int
	WalkSpeed       int
	AttackDelay     int
	Class           string
	MVP             bool
	Modes           string
	Source          string
	Maps            int // how many maps it spawns on, on search pages
}

func (m LookupMob) Custom() string { return customSource(m.Source) }

// Boss reports whether the monster is an MVP or a boss-class monster.
func (m LookupMob) Boss() bool { return m.MVP || m.Class == "Boss" }

// Drop is one of a monster's drops. Rates are out of 10000; Rate is with
// the server's drop rates, BaseRate as the monster database has it.
type Drop struct {
	MobID          int
	MobName        string
	MobLevel       int
	ItemID         int
	ItemName       string
	ItemType       string
	BaseRate       int
	Rate           int
	MVP            bool // an MVP prize, given to the player who earns the MVP
	StealProtected bool
}

func (d Drop) Chance() string     { return chance(d.Rate) }
func (d Drop) BaseChance() string { return chance(d.BaseRate) }

// Spawn is a group of a monster's spawns on one map.
type Spawn struct {
	Map    string
	Amount int
	Delay1 int // respawn time in ms
	Delay2 int // up to this much longer, at random
	Boss   bool
}

// Respawn says how long the monster takes to come back, when it isn't the
// usual few seconds.
func (s Spawn) Respawn() string {
	if s.Delay1 <= 5000 && s.Delay2 == 0 {
		return ""
	}
	r := duration(s.Delay1)
	if s.Delay2 > 0 {
		r += " to " + duration(s.Delay1+s.Delay2)
	}
	return r
}

// LookupInfo is wrapper_lookup_info: when the tables were filled and the
// rates the map server loaded.
type LookupInfo struct {
	LoadedAt time.Time
	Rates    map[string]int
}

// RatesChanged lists the rates that aren't the default 100%, for a note.
func (i LookupInfo) RatesChanged() []string {
	var out []string
	for _, r := range []struct{ key, name string }{
		{"base_exp_rate", "base exp"}, {"job_exp_rate", "job exp"},
		{"item_rate_common", "etc items"}, {"item_rate_heal", "healing items"}, {"item_rate_use", "usable items"},
		{"item_rate_equip", "equipment"}, {"item_rate_card", "cards"}, {"item_rate_mvp", "MVP prizes"},
	} {
		if v, ok := i.Rates[r.key]; ok && v != 100 {
			out = append(out, r.name+" "+strconv.Itoa(v)+"%")
		}
	}
	return out
}

// --- Store ---

func (s *store) lookupInfo(ctx context.Context) (LookupInfo, error) {
	info := LookupInfo{Rates: map[string]int{}}
	rows, err := s.db.QueryContext(ctx, "SELECT `name`, `value` FROM "+s.chars+"`wrapper_lookup_info`")
	if noSuchTable(err) {
		return info, errNoLookup
	}
	if err != nil {
		return info, err
	}
	defer rows.Close()
	for rows.Next() {
		var name, value string
		if err := rows.Scan(&name, &value); err != nil {
			return info, err
		}
		n, _ := strconv.ParseInt(value, 10, 64)
		if name == "loaded_at" {
			info.LoadedAt = time.Unix(n, 0)
		} else {
			info.Rates[name] = int(n)
		}
	}
	return info, rows.Err()
}

// likePattern matches q anywhere, with LIKE's wildcards taken literally.
func likePattern(q string) string {
	r := strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`)
	return "%" + r.Replace(q) + "%"
}

// searchItems finds items by name, AegisName or number, optionally of one
// type. It returns up to one more than a page, so the caller can tell
// whether there's a next page.
func (s *store) searchItems(ctx context.Context, q, itemType string, page int) ([]LookupItem, error) {
	query := "SELECT i.`id`, i.`name`, i.`type`, i.`subtype`, i.`weight`, i.`slots`, i.`source`," +
		" (SELECT COUNT(DISTINCT d.`mob_id`) FROM " + s.chars + "`wrapper_mob_drops` d WHERE d.`item_id` = i.`id`)" +
		" FROM " + s.chars + "`wrapper_items` i WHERE 1"
	var args []any
	if q != "" {
		query += " AND (i.`name` LIKE ? OR i.`aegis` LIKE ? OR i.`id` = ?)"
		args = append(args, likePattern(q), likePattern(q), q)
	}
	if itemType != "" {
		query += " AND i.`type` = ?"
		args = append(args, itemType)
	}
	query += " ORDER BY i.`name`, i.`id` LIMIT ? OFFSET ?"
	args = append(args, lookupPageSize+1, page*lookupPageSize)
	rows, err := s.db.QueryContext(ctx, query, args...)
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var items []LookupItem
	for rows.Next() {
		var i LookupItem
		if err := rows.Scan(&i.ID, &i.Name, &i.Type, &i.SubType, &i.Weight, &i.Slots, &i.Source, &i.DroppedBy); err != nil {
			return nil, err
		}
		items = append(items, i)
	}
	return items, rows.Err()
}

// itemTypes lists the item types there are, for the search filter.
func (s *store) itemTypes(ctx context.Context) ([]string, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT DISTINCT `type` FROM "+s.chars+"`wrapper_items` ORDER BY `type`")
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var types []string
	for rows.Next() {
		var t string
		if err := rows.Scan(&t); err != nil {
			return nil, err
		}
		types = append(types, t)
	}
	return types, rows.Err()
}

// lookupItem is one item, or nil if there's none with that number.
func (s *store) lookupItem(ctx context.Context, id int) (*LookupItem, error) {
	var i LookupItem
	err := s.db.QueryRowContext(ctx, "SELECT `id`, `name`, `aegis`, `type`, `subtype`, `buy`, `sell`, `weight`,"+
		" `attack`, `magic_attack`, `defense`, `range`, `slots`, `weapon_level`, `armor_level`,"+
		" `equip_level_min`, `equip_level_max`, `refineable`, `gender`, `jobs`, `locations`,"+
		" `script`, `equip_script`, `unequip_script`, `source` FROM "+s.chars+"`wrapper_items` WHERE `id` = ?", id).Scan(
		&i.ID, &i.Name, &i.Aegis, &i.Type, &i.SubType, &i.Buy, &i.Sell, &i.Weight,
		&i.Attack, &i.MagicAttack, &i.Defense, &i.Range, &i.Slots, &i.WeaponLevel, &i.ArmorLevel,
		&i.EquipLevelMin, &i.EquipLevelMax, &i.Refineable, &i.Gender, &i.Jobs, &i.Locations,
		&i.Script, &i.EquipScript, &i.UnEquipScript, &i.Source)
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &i, nil
}

const dropColumns = "d.`mob_id`, m.`name`, m.`level`, d.`item_id`, i.`name`, i.`type`, d.`base_rate`, d.`rate`, d.`mvp`, d.`steal_protected`"

func (s *store) drops(ctx context.Context, where string, id int) ([]Drop, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT "+dropColumns+" FROM "+s.chars+"`wrapper_mob_drops` d"+
		" JOIN "+s.chars+"`wrapper_mobs` m ON m.`id` = d.`mob_id`"+
		" JOIN "+s.chars+"`wrapper_items` i ON i.`id` = d.`item_id`"+
		" WHERE "+where, id)
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var drops []Drop
	for rows.Next() {
		var d Drop
		if err := rows.Scan(&d.MobID, &d.MobName, &d.MobLevel, &d.ItemID, &d.ItemName, &d.ItemType,
			&d.BaseRate, &d.Rate, &d.MVP, &d.StealProtected); err != nil {
			return nil, err
		}
		drops = append(drops, d)
	}
	return drops, rows.Err()
}

// droppedBy lists the monsters that drop an item, best chance first.
func (s *store) droppedBy(ctx context.Context, itemID int) ([]Drop, error) {
	return s.drops(ctx, "d.`item_id` = ? ORDER BY d.`rate` DESC, m.`level`, m.`name`", itemID)
}

// mobDrops lists a monster's drops in its own order, MVP prizes first.
func (s *store) mobDrops(ctx context.Context, mobID int) ([]Drop, error) {
	return s.drops(ctx, "d.`mob_id` = ? ORDER BY d.`mvp` DESC, d.`slot`", mobID)
}

// searchMobs finds monsters by name, AegisName or number; spawning keeps
// only those that spawn on a map. Returns up to one more than a page.
func (s *store) searchMobs(ctx context.Context, q string, spawning bool, page int) ([]LookupMob, error) {
	query := "SELECT m.`id`, m.`name`, m.`level`, m.`hp`, m.`base_exp`, m.`job_exp`, m.`race`, m.`element`," +
		" m.`element_level`, m.`size`, m.`class`, m.`mvp`, m.`source`," +
		" (SELECT COUNT(DISTINCT s.`map`) FROM " + s.chars + "`wrapper_mob_spawns` s WHERE s.`mob_id` = m.`id`) AS maps" +
		" FROM " + s.chars + "`wrapper_mobs` m WHERE 1"
	var args []any
	if q != "" {
		query += " AND (m.`name` LIKE ? OR m.`aegis` LIKE ? OR m.`id` = ?)"
		args = append(args, likePattern(q), likePattern(q), q)
	}
	if spawning {
		query += " AND EXISTS (SELECT 1 FROM " + s.chars + "`wrapper_mob_spawns` s WHERE s.`mob_id` = m.`id`)"
	}
	query += " ORDER BY m.`level`, m.`name`, m.`id` LIMIT ? OFFSET ?"
	args = append(args, lookupPageSize+1, page*lookupPageSize)
	rows, err := s.db.QueryContext(ctx, query, args...)
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var mobs []LookupMob
	for rows.Next() {
		var m LookupMob
		if err := rows.Scan(&m.ID, &m.Name, &m.Level, &m.HP, &m.BaseExp, &m.JobExp, &m.Race, &m.Element,
			&m.ElementLevel, &m.Size, &m.Class, &m.MVP, &m.Source, &m.Maps); err != nil {
			return nil, err
		}
		mobs = append(mobs, m)
	}
	return mobs, rows.Err()
}

// lookupMob is one monster, or nil if there's none with that number.
func (s *store) lookupMob(ctx context.Context, id int) (*LookupMob, error) {
	var m LookupMob
	err := s.db.QueryRowContext(ctx, "SELECT `id`, `name`, `aegis`, `level`, `hp`, `sp`, `base_exp`, `job_exp`,"+
		" `mvp_exp`, `attack`, `attack2`, `defense`, `magic_defense`, `resistance`, `magic_resistance`,"+
		" `str`, `agi`, `vit`, `int`, `dex`, `luk`, `attack_range`, `size`, `race`, `element`, `element_level`,"+
		" `walk_speed`, `attack_delay`, `class`, `mvp`, `modes`, `source` FROM "+s.chars+"`wrapper_mobs` WHERE `id` = ?", id).Scan(
		&m.ID, &m.Name, &m.Aegis, &m.Level, &m.HP, &m.SP, &m.BaseExp, &m.JobExp,
		&m.MvpExp, &m.Attack, &m.Attack2, &m.Defense, &m.MagicDefense, &m.Resistance, &m.MagicResistance,
		&m.Str, &m.Agi, &m.Vit, &m.Int, &m.Dex, &m.Luk, &m.AttackRange, &m.Size, &m.Race, &m.Element, &m.ElementLevel,
		&m.WalkSpeed, &m.AttackDelay, &m.Class, &m.MVP, &m.Modes, &m.Source)
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	if err != nil {
		return nil, err
	}
	return &m, nil
}

// mobSpawns lists where a monster spawns, the spawns on one map with the
// same respawn time added up.
func (s *store) mobSpawns(ctx context.Context, mobID int) ([]Spawn, error) {
	rows, err := s.db.QueryContext(ctx, "SELECT `map`, SUM(`amount`), `delay1`, `delay2`, MAX(`boss`) FROM "+s.chars+"`wrapper_mob_spawns`"+
		" WHERE `mob_id` = ? GROUP BY `map`, `delay1`, `delay2` ORDER BY SUM(`amount`) DESC, `map`", mobID)
	if noSuchTable(err) {
		return nil, errNoLookup
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var spawns []Spawn
	for rows.Next() {
		var sp Spawn
		if err := rows.Scan(&sp.Map, &sp.Amount, &sp.Delay1, &sp.Delay2, &sp.Boss); err != nil {
			return nil, err
		}
		spawns = append(spawns, sp)
	}
	return spawns, rows.Err()
}

// --- Pages ---

// lookupSearch is what the item and monster search pages share.
type lookupSearch struct {
	Path     string // the page's own path, for its links
	Q        string
	Page     int
	HasNext  bool
	Info     LookupInfo
	Renewal  bool
	Missing  bool // the tables aren't there yet
	Type     string
	Types    []string
	Spawning bool
	Items    []LookupItem
	Mobs     []LookupMob
}

// PageURL links to another page of the same search.
func (d lookupSearch) PageURL(page int) string {
	q := url.Values{}
	if d.Q != "" {
		q.Set("q", d.Q)
	}
	if d.Type != "" {
		q.Set("type", d.Type)
	}
	if d.Spawning {
		q.Set("spawning", "1")
	}
	if page > 0 {
		q.Set("page", strconv.Itoa(page))
	}
	if len(q) == 0 {
		return d.Path
	}
	return d.Path + "?" + q.Encode()
}

func (d lookupSearch) PrevPage() int { return d.Page - 1 }
func (d lookupSearch) NextPage() int { return d.Page + 1 }

func searchParams(r *http.Request) lookupSearch {
	q := r.URL.Query()
	d := lookupSearch{Q: strings.TrimSpace(q.Get("q"))}
	if len(d.Q) > 100 {
		d.Q = d.Q[:100]
	}
	if p, err := strconv.Atoi(q.Get("page")); err == nil && p > 0 && p < 10000 {
		d.Page = p
	}
	return d
}

func (a *app) itemsPage(req *request) error {
	d := searchParams(req.r)
	d.Path = "/items"
	d.Type = req.r.URL.Query().Get("type")
	d.Renewal = a.conn.Renewal
	ctx := req.r.Context()
	err := func() (err error) {
		if d.Info, err = a.store.lookupInfo(ctx); err != nil {
			return err
		}
		if d.Types, err = a.store.itemTypes(ctx); err != nil {
			return err
		}
		d.Items, err = a.store.searchItems(ctx, d.Q, d.Type, d.Page)
		return err
	}()
	if errors.Is(err, errNoLookup) {
		d.Missing = true
	} else if err != nil {
		return err
	}
	if len(d.Items) > lookupPageSize {
		d.Items, d.HasNext = d.Items[:lookupPageSize], true
	}
	a.render(req, http.StatusOK, "lookup_items", view{Title: "Items", Data: d})
	return nil
}

func (a *app) monstersPage(req *request) error {
	d := searchParams(req.r)
	d.Path = "/monsters"
	d.Spawning = req.r.URL.Query().Get("spawning") != ""
	d.Renewal = a.conn.Renewal
	ctx := req.r.Context()
	err := func() (err error) {
		if d.Info, err = a.store.lookupInfo(ctx); err != nil {
			return err
		}
		d.Mobs, err = a.store.searchMobs(ctx, d.Q, d.Spawning, d.Page)
		return err
	}()
	if errors.Is(err, errNoLookup) {
		d.Missing = true
	} else if err != nil {
		return err
	}
	if len(d.Mobs) > lookupPageSize {
		d.Mobs, d.HasNext = d.Mobs[:lookupPageSize], true
	}
	a.render(req, http.StatusOK, "lookup_monsters", view{Title: "Monsters", Data: d})
	return nil
}

type itemPageData struct {
	Item      *LookupItem
	DroppedBy []Drop
	Info      LookupInfo
	Renewal   bool
}

func (a *app) itemPage(req *request) error {
	id, err := strconv.Atoi(req.r.PathValue("id"))
	if err != nil {
		return a.lookupNotFound(req, "item")
	}
	ctx := req.r.Context()
	d := itemPageData{Renewal: a.conn.Renewal}
	err = func() (err error) {
		if d.Info, err = a.store.lookupInfo(ctx); err != nil {
			return err
		}
		if d.Item, err = a.store.lookupItem(ctx, id); err != nil || d.Item == nil {
			return err
		}
		d.DroppedBy, err = a.store.droppedBy(ctx, id)
		return err
	}()
	if errors.Is(err, errNoLookup) {
		return a.lookupMissing(req, "Items")
	}
	if err != nil {
		return err
	}
	if d.Item == nil {
		return a.lookupNotFound(req, "item")
	}
	a.render(req, http.StatusOK, "lookup_item", view{Title: d.Item.Name, Data: d})
	return nil
}

type monsterPageData struct {
	Mob     *LookupMob
	Drops   []Drop
	Spawns  []Spawn
	Info    LookupInfo
	Renewal bool
}

func (a *app) monsterPage(req *request) error {
	id, err := strconv.Atoi(req.r.PathValue("id"))
	if err != nil {
		return a.lookupNotFound(req, "monster")
	}
	ctx := req.r.Context()
	d := monsterPageData{Renewal: a.conn.Renewal}
	err = func() (err error) {
		if d.Info, err = a.store.lookupInfo(ctx); err != nil {
			return err
		}
		if d.Mob, err = a.store.lookupMob(ctx, id); err != nil || d.Mob == nil {
			return err
		}
		if d.Drops, err = a.store.mobDrops(ctx, id); err != nil {
			return err
		}
		d.Spawns, err = a.store.mobSpawns(ctx, id)
		return err
	}()
	if errors.Is(err, errNoLookup) {
		return a.lookupMissing(req, "Monsters")
	}
	if err != nil {
		return err
	}
	if d.Mob == nil {
		return a.lookupNotFound(req, "monster")
	}
	a.render(req, http.StatusOK, "lookup_monster", view{Title: d.Mob.Name, Data: d})
	return nil
}

func (a *app) lookupNotFound(req *request, what string) error {
	a.render(req, http.StatusNotFound, "error", view{Title: "Not found",
		Error: "This server has no such " + what + "."})
	return nil
}

func (a *app) lookupMissing(req *request, title string) error {
	a.render(req, http.StatusServiceUnavailable, "error", view{Title: title, Error: lookupMissingText})
	return nil
}

const lookupMissingText = "The item and monster lists fill in when the game's map server starts. Try again in a minute."

// --- Formatting ---

// chance writes a drop rate out of 10000 as a percentage, e.g. 0.01%.
func chance(rate int) string {
	s := strconv.FormatFloat(float64(rate)/100, 'f', 2, 64)
	s = strings.TrimRight(strings.TrimRight(s, "0"), ".")
	return s + "%"
}

// tenths writes a number stored in tenths, e.g. 7 for 70 and 0.5 for 5.
func tenths(n int) string {
	s := strconv.Itoa(n / 10)
	if n%10 != 0 {
		s += "." + strconv.Itoa(n%10)
	}
	return s
}

// duration writes milliseconds the way a respawn time is talked about,
// e.g. "1 hour 10 minutes".
func duration(ms int) string {
	secs := (ms + 500) / 1000
	h, m, sec := secs/3600, secs/60%60, secs%60
	var parts []string
	if h > 0 {
		parts = append(parts, plural(h, "hour"))
	}
	if m > 0 {
		parts = append(parts, plural(m, "minute"))
	}
	if sec > 0 || len(parts) == 0 {
		parts = append(parts, plural(sec, "second"))
	}
	return strings.Join(parts, " ")
}

func plural(n int, unit string) string {
	if n == 1 {
		return "1 " + unit
	}
	return strconv.Itoa(n) + " " + unit + "s"
}
