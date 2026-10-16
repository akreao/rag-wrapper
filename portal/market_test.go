package main

import (
	"net/http/httptest"
	"strings"
	"testing"
)

func offer(id int, name string, price int64, seller string, cards ...string) Offer {
	return Offer{Item: Item{ItemID: id, Name: name, Amount: 1, Identified: true, CardNames: cards}, Price: price, Seller: seller, Map: "prontera", X: 150, Y: 180}
}

func TestOfferMatches(t *testing.T) {
	o := offer(1101, "Sword", 100, "Ann", "Hydra Card")
	o.cards = [4]int{4035}
	for q, want := range map[string]bool{
		"":       true,
		"  ":     true,
		"sword":  true,
		"SWO":    true,
		"hydra":  true,
		"1101":   true,
		"4035":   true, // the Hydra Card in it
		"4036":   false,
		"110":    false,
		"knife":  false,
		" 1101 ": true,
	} {
		if got := o.Matches(q); got != want {
			t.Errorf("Matches(%q) = %v", q, got)
		}
	}
}

func TestFilterOffersOrder(t *testing.T) {
	offers := []Offer{
		offer(1101, "Sword", 300, "Ann"),
		offer(501, "Red Potion", 50, "Bob"),
		offer(1101, "Sword", 100, "Cat"),
		offer(1102, "sword", 10, "Dan"), // another item with the same name
	}
	seller := func(os []Offer) string {
		var s []string
		for _, o := range os {
			s = append(s, o.Seller)
		}
		return strings.Join(s, ",")
	}
	if got := seller(filterOffers(offers, "", false)); got != "Bob,Cat,Ann,Dan" {
		t.Errorf("selling: cheapest first, got %s", got)
	}
	if got := seller(filterOffers(offers, "", true)); got != "Bob,Ann,Cat,Dan" {
		t.Errorf("buying: best paid first, got %s", got)
	}
	if got := seller(filterOffers(offers, "sword", false)); got != "Cat,Ann,Dan" {
		t.Errorf("search: got %s", got)
	}
}

func TestMarketURL(t *testing.T) {
	d := marketData{}
	if d.URL(marketSelling) != "/market" || d.URL(marketBuying) != "/market?shops=buying" {
		t.Fatal(d.URL(marketSelling), d.URL(marketBuying))
	}
	d.Query = "red potion"
	if d.URL(marketSelling) != "/market?q=red+potion" || d.URL(marketBuying) != "/market?q=red+potion&shops=buying" {
		t.Fatal(d.URL(marketSelling), d.URL(marketBuying))
	}
}

func TestMarketTemplate(t *testing.T) {
	a, err := newApp(nil, 99, connectInfo{})
	if err != nil {
		t.Fatal(err)
	}
	o := offer(1101, "Sword", 1234567, "<Ann>", "Hydra Card")
	o.Refine, o.Shop = 7, "Cheap swords"
	for _, c := range []struct {
		d    marketData
		want []string
	}{
		{marketData{Shops: marketSelling, Offers: []Offer{o}, Total: 1}, []string{"7 Sword", "Hydra Card", "1,234,567 z", "Cheap swords", "&lt;Ann&gt;", "prontera (150, 180)", `class="current">For sale`}},
		{marketData{Shops: marketBuying, Query: "x"}, []string{"No buying stores are open", `name="shops" value="buying"`}},
		{marketData{Shops: marketSelling, Query: "knife", Total: 3}, []string{"No shop sells “knife” right now"}},
	} {
		w := httptest.NewRecorder()
		req := &request{w: w, r: httptest.NewRequest("GET", "/market", nil), sess: &session{}}
		a.render(req, 200, "market", view{Title: "Market", Data: c.d})
		for _, s := range c.want {
			if !strings.Contains(w.Body.String(), s) {
				t.Errorf("page lacks %q", s)
			}
		}
	}
}
