# Item mod

A starting point for custom items: a potion and a headgear, each with both halves an item needs. Rename the items, change their Ids and stats, and add your own. The full guide is "Custom items" in the mods folder's `README.md`.

- `db/item_db.yml`: the server half, what the item is and does. Ids 40000 to 49999 are for custom items.
- `client/SystemEN/itemInfo_C.lua`: the client half, its name, description and look, under the same Id. It goes out with the next client patch (the Client patch screen).
- `npc/item_shop.txt`: a shop in Prontera (143, 170) that sells both, to try them in game. Delete it when you don't need it.

Switch the mod on on the Mods screen, restart the game servers, and publish a client patch. Both items reuse looks the client already has (the Red Potion's and the Ribbon's), so they work without any sprite files.

## A look of your own

Put the files in `client/data/`, named after `identifiedResourceName`:

- `client/data/sprite/아이템/<name>.spr` and `.act`: the sprite on the ground.
- `client/data/texture/유저인터페이스/item/<name>.bmp`: the inventory icon (24×24, magenta `#FF00FF` for transparent).
- `client/data/texture/유저인터페이스/collection/<name>.bmp`: the larger picture in the item's description window (75×100).

For a headgear's look on the head, the client also needs the view sprite (`client/data/sprite/악세사리/남/남_<name>.spr` and `.act`, and `여/여_` for female characters) and an entry tying the `View` number to it in `accessoryid.lub` and `accname.lub` (`data/luafiles514/lua files/datainfo/`). Those two ship compiled inside the client's `data.grf`, so the app can't merge a mod's entries into them; a mod has to carry readable copies of both whole files. Reusing an existing look, as the Kit Ribbon does, needs none of this.
