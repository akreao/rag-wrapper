# Market page in the portal

A public page listing what players' shops sell and what their buying stores want (roadmap feature 29). Built in the PR that adds this file.

- **From the tables rAthena already fills.** The map server writes every open vending shop to `vendings` / `vending_items` and every buying store to `buyingstores` / `buyingstore_items`, not only autotrade ones (`vending.cpp`, `buyingstore.cpp`). It empties them when it starts and deletes a shop's rows when the shop closes, so they list what's open now. No schema change and no rAthena change.
- **Shops are listed only while their character is online.** Autotrading characters count as online. This hides rows a map server that crashed left behind until its next start, since the char server marks everyone offline when it starts and stops.
- **Two tabs: For sale (vending) and Wanted (buying stores).** Each is one table, one row per item: item with refine and cards, price, amount, shop title, seller, map and coordinates. Rows are sorted by item name, then price: cheapest first for sale, best paid first for wanted, so the best deal for an item is the first row.
- **Search by item.** The box matches part of the item's name, part of a card's name (find every shop selling something with a Hydra Card), or an exact item number. Names come from `wrapper_items` (see `item-monster-lookup.md`), like the admin character page; before that table exists, items show as "Item 1101".
- **Open to everyone, no log-in,** linked from the top bar after Rankings, like the other player pages. Admins' shops are listed too, like on Online: they're real shops players can buy from (Steven, 2026-10-16).
