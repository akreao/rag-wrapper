package main

import (
	"bytes"
	"strings"
	"testing"
	"time"
)

func TestLookupFormatting(t *testing.T) {
	for rate, want := range map[int]string{10000: "100%", 7000: "70%", 150: "1.5%", 25: "0.25%", 1: "0.01%", 0: "0%"} {
		if got := chance(rate); got != want {
			t.Errorf("chance(%d) = %q, want %q", rate, got, want)
		}
	}
	for n, want := range map[int]string{70: "7", 5: "0.5", 0: "0", 1234: "123.4"} {
		if got := tenths(n); got != want {
			t.Errorf("tenths(%d) = %q, want %q", n, got, want)
		}
	}
	for ms, want := range map[int]string{
		3600000: "1 hour", 4200000: "1 hour 10 minutes", 600000: "10 minutes", 90000: "1 minute 30 seconds", 5000: "5 seconds",
	} {
		if got := duration(ms); got != want {
			t.Errorf("duration(%d) = %q, want %q", ms, got, want)
		}
	}
	if got := (Spawn{Delay1: 5000}).Respawn(); got != "" {
		t.Errorf("a usual respawn should say nothing, got %q", got)
	}
	if got := (Spawn{Delay1: 3600000, Delay2: 600000}).Respawn(); got != "1 hour to 1 hour 10 minutes" {
		t.Errorf("MVP respawn = %q", got)
	}
}

func TestLikePatternTakesWildcardsLiterally(t *testing.T) {
	if got := likePattern(`50%_off\`); got != `%50\%\_off\\%` {
		t.Fatalf("likePattern = %q", got)
	}
}

func TestLookupPageURL(t *testing.T) {
	d := lookupSearch{Path: "/monsters", Q: "poring & co", Spawning: true}
	if got := d.PageURL(2); got != "/monsters?page=2&q=poring+%26+co&spawning=1" {
		t.Errorf("PageURL = %q", got)
	}
	if got := (lookupSearch{Path: "/items"}).PageURL(0); got != "/items" {
		t.Errorf("first page of everything = %q", got)
	}
}

func TestRatesChanged(t *testing.T) {
	info := LookupInfo{Rates: map[string]int{"base_exp_rate": 300, "job_exp_rate": 100, "item_rate_card": 50}}
	if got := strings.Join(info.RatesChanged(), ", "); got != "base exp 300%, cards 50%" {
		t.Errorf("RatesChanged = %q", got)
	}
}

// The lookup pages render with every kind of row, so a template mistake
// fails here rather than on a player's screen.
func TestLookupPagesRender(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{Renewal: true})
	if err != nil {
		t.Fatal(err)
	}
	info := LookupInfo{LoadedAt: time.Unix(1700000000, 0), Rates: map[string]int{"base_exp_rate": 200}}
	potion := LookupItem{ID: 64999, Name: "Smoke Test Potion", Aegis: "Smoke_Test_Potion", Type: "Healing", Weight: 10,
		Script: "itemheal 10,0;", Source: "smoke-test", DroppedBy: 1, Jobs: "All"}
	sword := LookupItem{ID: 1101, Name: "Sword", Type: "Weapon", SubType: "1hSword", Slots: 3, Attack: 25,
		WeaponLevel: 1, Refineable: true, Gender: "Both", Jobs: "Swordman", Locations: "Right_Hand", EquipScript: "bonus bStr,1;"}
	drop := Drop{MobID: 1002, MobName: "Poring", MobLevel: 1, ItemID: 64999, ItemName: "Smoke <Test> Potion", ItemType: "Healing",
		BaseRate: 5000, Rate: 10000, MVP: true, StealProtected: true}
	poring := LookupMob{ID: 1002, Name: "Poring", Level: 1, HP: 55, BaseExp: 300, Race: "Plant", Element: "Water",
		ElementLevel: 1, Size: "Medium", Class: "Boss", Maps: 3, Source: "import"}
	pages := []struct {
		name string
		data any
		want []string
	}{
		{"lookup_items", lookupSearch{Path: "/items", Q: "potion", Info: info, Renewal: true, Types: []string{"Healing", "Weapon"},
			Type: "Healing", Items: []LookupItem{potion, sword}, HasNext: true, Page: 1},
			[]string{`href="/items/64999"`, "Mod: smoke-test", "Weapon (1hSword)", `href="/items?page=2&amp;q=potion&amp;type=Healing"`, `href="/items?q=potion&amp;type=Healing">Previous`, "base exp 200%", "level penalty", "<option selected>Healing</option>"}},
		{"lookup_items", lookupSearch{Path: "/items", Missing: true}, []string{"fill in when the game"}},
		{"lookup_monsters", lookupSearch{Path: "/monsters", Spawning: true, Info: info, Mobs: []LookupMob{poring}},
			[]string{`href="/monsters/1002"`, "Boss", "Custom", "checked"}},
		{"lookup_item", itemPageData{Item: &potion, DroppedBy: []Drop{drop}, Info: info, Renewal: true},
			[]string{"Smoke Test Potion", "itemheal 10,0;", "100%", "MVP prize", "Can't be stolen", `href="/monsters/1002"`}},
		{"lookup_item", itemPageData{Item: &sword, Info: info}, []string{"Sword [3]", "Swordman", "Right_Hand", "When equipped", "No monster drops this item."}},
		{"lookup_monster", monsterPageData{Mob: &poring, Drops: []Drop{drop}, Info: info, Renewal: true,
			Spawns: []Spawn{{Map: "prt_fild08", Amount: 70, Delay1: 5000}, {Map: "moc_pryd04", Amount: 1, Delay1: 3600000, Boss: true}}},
			[]string{"Smoke &lt;Test&gt; Potion", "prt_fild08", "1 hour", "Plant"}},
		{"lookup_monster", monsterPageData{Mob: &poring, Info: info}, []string{"drops nothing", "doesn't spawn"}},
	}
	for _, p := range pages {
		var buf bytes.Buffer
		if err := a.pages[p.name].ExecuteTemplate(&buf, "layout", view{Title: "x", Data: p.data}); err != nil {
			t.Fatalf("%s: %v", p.name, err)
		}
		for _, w := range p.want {
			if !strings.Contains(buf.String(), w) {
				t.Errorf("%s has no %q:\n%s", p.name, w, buf.String())
			}
		}
	}
}

// The Market page links its items and their cards to the item pages.
func TestMarketLinksToItemPages(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{})
	if err != nil {
		t.Fatal(err)
	}
	offer := Offer{Item: Item{ItemID: 1101, Name: "Sword", Refine: 7, Identified: true, cards: [4]int{4001, 0, 4002, 0},
		CardNames: []string{"Poring Card", "Fabre Card"}}, Price: 1000, Shop: "Swords", Seller: "Bob", Map: "prontera"}
	var buf bytes.Buffer
	if err := a.pages["market"].ExecuteTemplate(&buf, "layout", view{Title: "Market", Data: marketData{Shops: marketSelling, Offers: []Offer{offer}}}); err != nil {
		t.Fatal(err)
	}
	for _, w := range []string{`<a href="/items/1101">&#43;7 Sword</a>`, `<a href="/items/4001">Poring Card</a>, <a href="/items/4002">Fabre Card</a>`} {
		if !strings.Contains(buf.String(), w) {
			t.Errorf("market page has no %q:\n%s", w, buf.String())
		}
	}
}
