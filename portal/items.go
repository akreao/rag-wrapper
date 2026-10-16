package main

import (
	"context"
	"strconv"
	"strings"
)

// Special values of card0 from rAthena's itemdb.hpp: the card slots of
// forged and brewed items and of pet eggs hold other data, not cards.
const (
	card0Forge  = 0x00FF
	card0Create = 0x00FE
	card0Pet    = 0x0100
)

// Item is a row of rAthena's inventory table, with names looked up.
type Item struct {
	ItemID     int
	Name       string
	Amount     int
	Equipped   bool
	Identified bool
	Broken     bool
	Refine     int
	cards      [4]int
	CardNames  []string
}

// Label is the item as the game would show it, e.g. "+7 Blade".
func (i Item) Label() string {
	if i.Refine > 0 {
		return "+" + strconv.Itoa(i.Refine) + " " + i.Name
	}
	return i.Name
}

func (i Item) cardIDs() []int {
	switch i.cards[0] {
	case card0Forge, card0Create, card0Pet:
		return nil
	}
	var ids []int
	for _, c := range i.cards {
		if c != 0 {
			ids = append(ids, c)
		}
	}
	return ids
}

// Card is a card in an item, with its name, for linking to its page.
type Card struct {
	ID   int
	Name string
}

// Cards pairs CardNames with the cards' numbers (0 where unknown).
func (i Item) Cards() []Card {
	ids := i.cardIDs()
	cards := make([]Card, len(i.CardNames))
	for n, name := range i.CardNames {
		cards[n].Name = name
		if n < len(ids) {
			cards[n].ID = ids[n]
		}
	}
	return cards
}

// inventory lists what a character carries, equipped items first.
func (s *store) inventory(ctx context.Context, charID int) ([]Item, error) {
	return s.items(ctx, "SELECT "+itemColumns+" FROM "+s.chars+"`inventory` WHERE `char_id` = ? ORDER BY `equip` = 0, `nameid`, `id`", charID)
}

// cart lists what's in a character's cart (merchant classes only).
func (s *store) cart(ctx context.Context, charID int) ([]Item, error) {
	return s.items(ctx, "SELECT "+itemColumns+" FROM "+s.chars+"`cart_inventory` WHERE `char_id` = ? ORDER BY `nameid`, `id`", charID)
}

// storage lists an account's Kafra storage, shared by its characters.
func (s *store) storage(ctx context.Context, accountID int) ([]Item, error) {
	return s.items(ctx, "SELECT "+itemColumns+" FROM "+s.chars+"`storage` WHERE `account_id` = ? ORDER BY `nameid`, `id`", accountID)
}

// The inventory, cart_inventory and storage tables share these columns.
const itemColumns = "`nameid`, `amount`, `equip`, `identify`, `attribute`, `refine`, `card0`, `card1`, `card2`, `card3`"

func (s *store) items(ctx context.Context, query string, id int) ([]Item, error) {
	rows, err := s.db.QueryContext(ctx, query, id)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var items []Item
	for rows.Next() {
		var it Item
		var equip int64
		var identify, attribute int
		if err := rows.Scan(&it.ItemID, &it.Amount, &equip, &identify, &attribute, &it.Refine,
			&it.cards[0], &it.cards[1], &it.cards[2], &it.cards[3]); err != nil {
			return nil, err
		}
		it.Equipped = equip != 0
		it.Identified = identify != 0
		it.Broken = attribute != 0
		items = append(items, it)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	ptrs := make([]*Item, len(items))
	for i := range items {
		ptrs[i] = &items[i]
	}
	if err := s.nameItems(ctx, ptrs); err != nil {
		return nil, err
	}
	return items, nil
}

// nameItems fills in the names of items and their cards.
func (s *store) nameItems(ctx context.Context, items []*Item) error {
	ids := map[int]bool{}
	for _, it := range items {
		ids[it.ItemID] = true
		for _, c := range it.cardIDs() {
			ids[c] = true
		}
	}
	names, err := s.itemNames(ctx, ids)
	if err != nil {
		return err
	}
	name := func(id int) string {
		if n, ok := names[id]; ok {
			return n
		}
		return "Item " + strconv.Itoa(id)
	}
	for _, it := range items {
		it.Name = name(it.ItemID)
		it.CardNames = nil
		for _, c := range it.cardIDs() {
			it.CardNames = append(it.CardNames, name(c))
		}
	}
	return nil
}

// itemNames looks up item names in wrapper_items, which the rAthena image
// fills from the server's item database when the map server starts (see
// lookup.go). Until it has, there are no names, and pages show item
// numbers instead.
func (s *store) itemNames(ctx context.Context, ids map[int]bool) (map[int]string, error) {
	names := map[int]string{}
	if len(ids) == 0 {
		return names, nil
	}
	args := make([]any, 0, len(ids))
	for id := range ids {
		args = append(args, id)
	}
	rows, err := s.db.QueryContext(ctx,
		"SELECT `id`, `name` FROM "+s.chars+"`wrapper_items` WHERE `id` IN (?"+strings.Repeat(", ?", len(args)-1)+")", args...)
	if noSuchTable(err) {
		return names, nil
	}
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	for rows.Next() {
		var id int
		var n string
		if err := rows.Scan(&id, &n); err != nil {
			return nil, err
		}
		names[id] = n
	}
	return names, rows.Err()
}
