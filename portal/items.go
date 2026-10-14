package main

import (
	"context"
	"errors"
	"strconv"
	"strings"

	"github.com/go-sql-driver/mysql"
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

// inventory lists a character's items, equipped ones first.
func (s *store) inventory(ctx context.Context, charID int) ([]Item, error) {
	rows, err := s.db.QueryContext(ctx,
		"SELECT `nameid`, `amount`, `equip`, `identify`, `attribute`, `refine`, `card0`, `card1`, `card2`, `card3` "+
			"FROM `inventory` WHERE `char_id` = ? ORDER BY `equip` = 0, `nameid`, `id`", charID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var items []Item
	ids := map[int]bool{}
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
		ids[it.ItemID] = true
		for _, c := range it.cardIDs() {
			ids[c] = true
		}
		items = append(items, it)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}

	names, err := s.itemNames(ctx, ids)
	if err != nil {
		return nil, err
	}
	name := func(id int) string {
		if n, ok := names[id]; ok {
			return n
		}
		return "Item " + strconv.Itoa(id)
	}
	for i := range items {
		items[i].Name = name(items[i].ItemID)
		for _, c := range items[i].cardIDs() {
			items[i].CardNames = append(items[i].CardNames, name(c))
		}
	}
	return items, nil
}

// itemNames looks up item names in wrapper_item_names, which the rAthena
// image's setup step fills from the item database it was built with. Before
// that has run (an image from before this table existed) there are no
// names, and pages show item numbers instead.
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
		"SELECT `id`, `name` FROM `wrapper_item_names` WHERE `id` IN (?"+strings.Repeat(", ?", len(args)-1)+")", args...)
	var me *mysql.MySQLError
	if errors.As(err, &me) && me.Number == 1146 { // ER_NO_SUCH_TABLE
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
