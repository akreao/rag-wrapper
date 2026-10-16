# Docker stack audit fixes

Fixes for the Docker stack findings of the 2026-10-16 audit (fix list items 3, 4, 8 and 12).

## Game mode: checked at start-up

The image writes the `PRERE` it was compiled with to `/rathena/BUILD_PRERE`. `entrypoint.sh` compares it with the `PRERE` it's started with and stops (exit 66) when they differ, before touching the database.

- Catches a mode switch whose rebuild or image pull failed, was cancelled or never ran (offline, or `docker compose up` without `--build`). Before, the old binary ran against the other mode's characters database.
- The local tag stays `rag-wrapper/rathena:${PACKETVER}`, so the desktop app's image handling doesn't change.
- Images built before this have no file and aren't checked.
- `RATHENA_COMMIT` isn't checked the same way: a pin change without a rebuild runs an older but working server, and the database check for upgrades from a newer version already covers the harmful direction.

## Failed database upgrades: stop, don't re-run

`wrapper_sql_upgrades` has a `done` column (added with `ADD COLUMN IF NOT EXISTS`, existing rows count as done). An upgrade file is recorded with `done=0` before it runs and set to `done=1` after.

- When a file fails, or a later start finds a `done=0` row, `db-init` stops (exit 67) and names the file and the database. Before, the next start re-ran the file from the top and failed on its first, already-applied statement, with a message that didn't say what happened.
- The way out is the backup the desktop app makes before every server update. Someone who fixed the database by hand marks the row done with the `UPDATE` the message prints.
- The first start records all current files in one `INSERT`, so an interrupted first start doesn't leave a half-recorded baseline that makes the next start run the rest for real.
- The tracking queries are assigned to variables first, so a failed query stops the script instead of reading as "not applied".

## Logs: narrower item logging, capped container logs

- `entrypoint.sh` writes `enable_logs: 0xFFFFFF73` into the `log_conf.txt` import: everything rAthena logs by default except (P) player drops and pickups, (L) monster loot and (C) consumed items (every potion, every arrow). Those were most of the rows in tables nothing trims, and every backup copies them. Trades, vending, storage, mail, MVP prizes, NPC and script items, GM commands and the rest stay logged.
- It's written before the overrides folder's `log_conf.txt`, so a server can turn them back on there.
- No pruning of old rows for now; the narrower default removes most of the growth.
- Player drops and pickups stay unlogged (Steven, 2026-10-16). Items passed by dropping them can't be traced, which rarely matters on a friends server, and the rows were a large share of the growth.
- Every service logs with Docker's `json-file` driver capped at 3 files of 10 MB (`x-logging` in `compose.yaml`).

## Ports: optional bind address

`BIND_IP` in `.env` (empty by default) puts an address in front of every published port (login, char, map, web and the portal).

- Empty keeps listening on every address, as before, so nothing changes for existing installs.
- Set to the machine's LAN or VPN address, the server isn't reachable from any other network the machine joins. Docker's port rules go around ufw and firewalld, so the host firewall can't do this.
- IPv4 only. When the machine doesn't have that address (DHCP gave it a new one), `docker compose up` fails to publish the ports, so an app setting for it should follow `PUBLIC_IP` and the address-change warning.
- A client on the server machine itself must then use that address, not 127.0.0.1.
- The desktop app gets an opt-in Settings switch, off by default, that sets `BIND_IP` to the chosen server address (Steven, 2026-10-16). Off keeps listening everywhere, so a changed address never stops a start nobody expected.

## Example passwords: warned about, not refused

Audit stack item 7. `.env.example`'s `DB_PASSWORD=change-me`, `DB_ROOT_PASSWORD=change-me-too` and `INTER_PASSWORD=changeme123` are public. `entrypoint.sh` prints a `WARNING:` line for each one still in use, at every `db-init` and in `config`.

- It doesn't refuse to start. A server that has run on them for months would stop, and the database passwords can't simply be changed in `.env`: MariaDB keeps the ones the volume was made with.
- The desktop app shows the same warning on the dashboard (`notices.rs`), since nobody reads db-init's log.
- `INTER_PASSWORD` has a "Change INTER_PASSWORD" button. It's safe: `db-init` sets account 1 to it on every start, before login, char and map read it. A running server is started again in full (the `start` task), never just the game servers, which would get a password the database doesn't have yet.
- The database passwords' warning has no fix, only "Got it" (`PLACEHOLDER_DB_PASSWORDS_NOTED=yes`). The database port isn't published, so they matter less.
- A new `.env` made next to an existing database now gets a random `INTER_PASSWORD` too. Only the database passwords have to stay.

## Admin account: say when it isn't made

Audit stack item 12. `db-init` still makes the admin account only when the name is free, and never changes an existing account's password or group: it may have been changed on the portal since.

- It now logs a line when it leaves one alone: `Admin account kept: NAME ...` (another password, or not in group 99) or `Admin account not created: NAME ...` (the name is `INTER_USER`'s). The same password and group logs nothing new.
- After a start that handed `db-init` an `ADMIN_PASSWORD`, the app reads db-init's log (the last run only) before it blanks the password, and keeps what it found in `ADMIN_NOTICE`. The dashboard says what happened and where to change the password, until "Got it" or a new setup.
- Reading the log rather than the database keeps the check in one place (`entrypoint.sh`), and needs no database password in the app.
- Setup refuses an admin name equal to `INTER_USER`.

## Local builds: compile jobs by memory

Audit stack item 13. The Dockerfile runs one `make` job per 2 GB of `MemAvailable`, at least 1 and at most one per core. g++ takes 1 to 2 GB on rAthena's largest files, so one job per core could run out of memory on a machine with many cores and little memory.

- The `MAKE_JOBS` build arg (and `.env` setting, through `compose.yaml`) overrides it. Empty picks by memory.
- Layer order is unchanged. Changing the Dockerfile changes the build files' hash, so existing installs download or rebuild the image once.

## Item and monster lookup and rate events: second audit round

Release fix list items 4, 5, 6, 7 and 13. The details are in [item-monster-lookup.md](item-monster-lookup.md) and [rate-events.md](rate-events.md).

- Commented-out `//` spawn lines are no longer stored as spawns (maps like "//gefenia01" on the Monsters pages, monsters that never spawn listed as spawning).
- The lookup tables are loaded with `--default-character-set=utf8mb4`, so names that aren't ASCII aren't garbled. Only that load: the schema and upgrade imports are left as they were.
- A mod's partial `Modes`, `RaceGroups` or `Locations` changes those keys only, as rAthena does, so an MVP made aggressive stays an MVP on the pages. `Ai` resets `Modes`; `Jobs` is still replaced whole.
- The rate events script keeps the current rates when its query fails.
- The lookup load is skipped when a hash of everything it can read matches the one stored with the tables, and runs under `nice` otherwise. When in doubt it loads.
- `mod_files()`, unused since the lookup load replaced the item names load, is gone.

