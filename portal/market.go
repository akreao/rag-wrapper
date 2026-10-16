package main

import (
	"context"
	"net/http"
	"net/url"
	"sort"
	"strconv"
	"strings"
)

// The market page: what players' vending shops sell and what their buying
// stores want, from the tables the map server keeps for every open shop
// (vendings and buyingstores). The map server empties them when it starts
// and each shop's row goes when it closes, so they list the shops open now.

// Market tabs, as the ?shops= value.
const (
	marketSelling = "selling"
	marketBuying  = "buying"
)

// Offer is one item in an open shop.
type Offer struct {
	Item
	Price  int64
	Shop   string
	Seller string
	Map    string
	X, Y   int
}

// Matches says whether the offer is for what the search names: the item's
// or one of its cards' numbers, or part of the item's or a card's name.
func (o Offer) Matches(q string) bool {
	q = strings.TrimSpace(q)
	if q == "" {
		return true
	}
	if id, err := strconv.Atoi(q); err == nil {
		if o.ItemID == id {
			return true
		}
		for _, c := range o.cardIDs() {
			if c == id {
				return true
			}
		}
		return false
	}
	q = strings.ToLower(q)
	if strings.Contains(strings.ToLower(o.Name), q) {
		return true
	}
	for _, c := range o.CardNames {
		if strings.Contains(strings.ToLower(c), q) {
			return true
		}
	}
	return false
}

// Shops are only listed while their character is online (an autotrading
// character counts as online), so a map server that stopped without
// clearing its tables doesn't leave old shops on the page.

// vendingOffers lists what vending shops sell. Their items are in the
// seller's cart, which holds the refine and cards.
func (s *store) vendingOffers(ctx context.Context) ([]Offer, error) {
	return s.offers(ctx, "SELECT ci.`nameid`, vi.`amount`, ci.`identify`, ci.`attribute`, ci.`refine`,"+
		" ci.`card0`, ci.`card1`, ci.`card2`, ci.`card3`, vi.`price`, v.`title`, c.`name`, v.`map`, v.`x`, v.`y`"+
		" FROM "+s.chars+"`vendings` v"+
		" JOIN "+s.chars+"`vending_items` vi ON vi.`vending_id` = v.`id`"+
		" JOIN "+s.chars+"`cart_inventory` ci ON ci.`id` = vi.`cartinventory_id`"+
		" JOIN "+s.chars+"`char` c ON c.`char_id` = v.`char_id`"+
		" WHERE c.`online` <> 0")
}

// buyingOffers lists what buying stores want. They name a plain item, with
// no refine or cards.
func (s *store) buyingOffers(ctx context.Context) ([]Offer, error) {
	return s.offers(ctx, "SELECT bi.`item_id`, bi.`amount`, 1, 0, 0, 0, 0, 0, 0, bi.`price`, b.`title`, c.`name`, b.`map`, b.`x`, b.`y`"+
		" FROM "+s.chars+"`buyingstores` b"+
		" JOIN "+s.chars+"`buyingstore_items` bi ON bi.`buyingstore_id` = b.`id`"+
		" JOIN "+s.chars+"`char` c ON c.`char_id` = b.`char_id`"+
		" WHERE c.`online` <> 0")
}

func (s *store) offers(ctx context.Context, query string) ([]Offer, error) {
	rows, err := s.db.QueryContext(ctx, query)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var offers []Offer
	for rows.Next() {
		var o Offer
		var identify, attribute int
		if err := rows.Scan(&o.ItemID, &o.Amount, &identify, &attribute, &o.Refine,
			&o.cards[0], &o.cards[1], &o.cards[2], &o.cards[3],
			&o.Price, &o.Shop, &o.Seller, &o.Map, &o.X, &o.Y); err != nil {
			return nil, err
		}
		o.Identified = identify != 0
		o.Broken = attribute != 0
		offers = append(offers, o)
	}
	if err := rows.Err(); err != nil {
		return nil, err
	}
	ptrs := make([]*Item, len(offers))
	for i := range offers {
		ptrs[i] = &offers[i].Item
	}
	if err := s.nameItems(ctx, ptrs); err != nil {
		return nil, err
	}
	return offers, nil
}

type marketData struct {
	Shops  string // marketSelling or marketBuying
	Query  string
	Offers []Offer
	Total  int // offers before the search
}

// URL links to a tab, keeping the search.
func (d marketData) URL(shops string) string {
	q := url.Values{}
	if shops != marketSelling {
		q.Set("shops", shops)
	}
	if d.Query != "" {
		q.Set("q", d.Query)
	}
	if len(q) == 0 {
		return "/market"
	}
	return "/market?" + q.Encode()
}

// filterOffers keeps the offers the search matches, sorted by item and
// then price: cheapest first when selling, best paid first when buying.
func filterOffers(offers []Offer, q string, buying bool) []Offer {
	var kept []Offer
	for _, o := range offers {
		if o.Matches(q) {
			kept = append(kept, o)
		}
	}
	sort.SliceStable(kept, func(i, j int) bool {
		a, b := kept[i], kept[j]
		if a.Name != b.Name {
			return strings.ToLower(a.Name) < strings.ToLower(b.Name)
		}
		if a.ItemID != b.ItemID {
			return a.ItemID < b.ItemID
		}
		if a.Price != b.Price {
			return (a.Price < b.Price) != buying
		}
		return a.Seller < b.Seller
	})
	return kept
}

func (a *app) market(req *request) error {
	q := req.r.URL.Query()
	d := marketData{Shops: marketSelling, Query: strings.TrimSpace(q.Get("q"))}
	if r := []rune(d.Query); len(r) > 100 {
		d.Query = string(r[:100])
	}
	if q.Get("shops") == marketBuying {
		d.Shops = marketBuying
	}
	var offers []Offer
	var err error
	if d.Shops == marketBuying {
		offers, err = a.store.buyingOffers(req.r.Context())
	} else {
		offers, err = a.store.vendingOffers(req.r.Context())
	}
	if err != nil {
		return err
	}
	d.Total = len(offers)
	d.Offers = filterOffers(offers, d.Query, d.Shops == marketBuying)
	a.render(req, http.StatusOK, "market", view{Title: "Market", Data: d})
	return nil
}
