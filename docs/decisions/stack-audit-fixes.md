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
