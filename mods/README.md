# Mods

Server-side custom content: NPC scripts and rAthena database changes. The rAthena containers see this folder, read-only, as `mods/`, and on every start `entrypoint.sh` loads the mods named in `enabled.txt`. The desktop app's Mods screen lists the folders here and switches them on and off by editing that file.

## A mod

One folder per mod, named with letters, digits, `.`, `_` and `-` only:

```
mods/
  enabled.txt
  warper/
    npc/warper.txt
  custom-items/
    npc/item_shop.txt
    db/item_db.yml
    db/pre-re/mob_db.yml
```

- **`npc/`**: every `.txt` file in it (subfolders too) is loaded as an NPC script, in path order, after rAthena's own scripts. `@reloadscript` keeps them.
- **`db/`**: YAML files named like rAthena's `db/import/` files (`item_db.yml`, `mob_db.yml`, `quest_db.yml`, ...), in the same format, `Header` included. They're loaded after rAthena's own entries and `db/import`, so an entry with an existing `Id` changes it and a new one adds to it. Files in `db/pre-re/` or `db/re/` are only loaded in that game mode.
- Nothing for the game client: sprites, item names in the client and other client files go out through the desktop app's Client patch screen.

## Turning mods on

`enabled.txt` has one mod folder name per line; `#` starts a comment. Mods load in the order of this file. The Mods screen's arrows reorder it, and a mod switched on there goes to the end.

Changes apply when the game servers restart; no rebuild is needed. The database is backed up first when the restart loads a mod's `db/` files for the first time.

## When two mods change the same entry

Their changes combine. rAthena applies a mod's entry as changes to what's already loaded, so fields one mod leaves out keep the other's value, and lists combine: a monster's `Drops` (without an `Index`) gain both mods' drops (a drop both add is kept once, from the later mod), keyed lists such as a skill tree's skills or an item group's sub-groups keep both mods' entries, and maps like an item's `Jobs` or `Flags` get both mods' keys.

When both set the same value differently (an item's `Buy`, the drop at one `Drops` `Index`, the same job in `Jobs`), the mod later in `enabled.txt` wins, and the Mods screen lists the clash under both mods as soon as the files change (it loads on the next restart). `const.yml` is the exception: rAthena keeps a constant's first value. Clashes never stop the server.

Two details, from how rAthena reads each database:
- An item's `Jobs` replaces the jobs it had rather than adding to them, so where more than one mod sets `Jobs` on the same item, the server combines them in a generated `db/import/mods-merged/item_db.yml`, loaded after the mods. Like any import, a mod's `Jobs` still replaces rAthena's own jobs for that item.
- A few lists are replaced whole, and each mod's counts as one value: `Probability` in `abra_db.yml`, `Random` and a slot's `Options` in `item_randomopt_group.yml`, and `Order` in `item_enchant.yml`.

`Header: Clear: true` in a mod's file empties that database, rAthena's own entries and earlier mods' included, before the file loads; the log warns about it.

## Problems

The map server's log names the file of each script or database error, and the Mods screen shows the errors and warnings from each mod's files under it. A file in `db/` that isn't named after one of rAthena's databases is skipped with a warning.
