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

`enabled.txt` has one mod folder name per line; `#` starts a comment. Mods load in the order of this file, so when two change the same entry the later one wins. The Mods screen's arrows reorder it, and a mod switched on there goes to the end.

Changes apply when the game servers restart; no rebuild is needed. The database is backed up first when the restart brings in a mod with `db/` files, or changed ones.

## Problems

The map server's log names the file of each script or database error, and the Mods screen shows the errors and warnings from each mod's files under it. A file in `db/` that isn't named after one of rAthena's databases is skipped with a warning.
