# rag-wrapper

A Linux desktop app that runs a private rAthena server and its database, for use with the real Ragnarok Online client.

## Desktop app

The app in [`desktop/`](desktop/README.md) runs the stack for you: first-run admin setup, start and stop, build progress, and each service's status and logs.

It ships as an AppImage on the [releases page](https://github.com/akreao/rag-wrapper/releases). It needs Docker with the Compose plugin 2.24 or newer (your user in the `docker` group) and FUSE 2 (`libfuse2`, or `libfuse2t64` on Ubuntu 24.04 and later):

```sh
chmod +x rag-wrapper-*-x86_64.AppImage
./rag-wrapper-*-x86_64.AppImage
```

After the first run it's in your applications menu too. To update, download the new AppImage and run it instead of the old one; the server's settings, database, backups and client files stay where they are.

## Running the server stack

The server side runs in Docker: MariaDB, rAthena's login, char, map and web servers built from source for your client's packet version, and the account portal.

```sh
cp .env.example .env   # then change the passwords
docker compose up -d --build
docker compose logs -f login char map
```

The first build compiles rAthena and takes several minutes. The desktop app downloads prebuilt images instead when they're published for its settings (`PREBUILT_IMAGES` in `.env`); `docker compose` on its own always builds. On first start, `db-init` creates the database and imports rAthena's schema; later starts skip the import.

| Port | Server |
|------|--------|
| 6900 | login (point your client's `clientinfo.xml` here) |
| 6121 | char |
| 5121 | map |
| 8888 | web (emblems and settings for newer clients; the portal's `ExternalSettings_kr.lub` points clients here) |
| 8080 | account portal (`PORTAL_PORT`) |

Settings in `.env`:
- `PUBLIC_IP` is the address the client is sent for the char and map servers. Keep `127.0.0.1` when the client runs on this machine; use the machine's LAN IP for other computers.
- `PACKETVER` must match your client's date. Changing it needs a rebuild (`docker compose build`).
- `PRERE` picks Pre-Renewal (`yes`) or Renewal (`no`). It's compiled in, so changing it needs a rebuild too; the servers refuse to start on an image built for the other mode.
- `BIND_IP`, when set to one of this machine's IPv4 addresses (its LAN or VPN address), makes the ports listen on that address only. Empty listens on all of them. Docker's port rules go around ufw and firewalld, so this, not the host firewall, is what keeps the server off other networks.
- `RATHENA_COMMIT` is the rAthena commit to build, from the `rag-wrapper` branch of `akreao/rathena`. Change it to pick up server changes, then rebuild. A new app version that moves the pin in `.env.example` offers the new commit as a server update; the desktop app applies it when asked, and Settings can go back to the previous one. `RATHENA_COMMIT_PREVIOUS` is the commit the app's previous release came with: when you move `RATHENA_COMMIT`, set it to the old value. `RATHENA_REPO` points at a different fork.

Gameplay settings (rates, server name, message of the day and anything else in rAthena's configs) go in [`overrides/`](overrides/README.md) as rAthena `conf/import`-style files; restart the game servers (`docker compose up -d --force-recreate login char map`) to apply them. The desktop app's Settings screen writes the common ones for you.

Server-side mods (NPC scripts and rAthena database changes) go in [`mods/`](mods/README.md), one folder per mod, switched on by name, in load order, in `mods/enabled.txt`; they load when the game servers restart. The desktop app's Mods screen lists them and switches them on and off. It also switches on the custom NPCs that come with rAthena (`npc/custom/`), written to `overrides/custom_npcs.txt`.

`docker compose down` stops everything; the database lives in the `db-data` volume and survives restarts. `docker compose down -v` deletes it.

## Account portal

Players create and manage their accounts at `http://<server's LAN IP>:8080`. The game client's own `_M`/`_F` sign-up is off, so this is the way in.

- **Anyone on the LAN** can register (username, password, email, birthdate, sex), then log in to change their password, email and birthdate and see their characters. The game asks for the birthdate when deleting a character.
- **Admins** (group 99, such as the first admin created on setup) also get an Admin page: search accounts by username, email or character name, change an account's group (GM level), ban for a set time or until lifted, and set a new password for a player who forgot theirs. Changes apply at the player's next game login; a player already in game stays on until they log out. Each character has its own page with its zeny, position, inventory and cart (read-only), the account page shows the account's storage, and buttons that move a logged-out character stuck somewhere back to its save point, or to Prontera when the save point itself is the problem. Item names come from the item and monster lookup's tables (below).
- **How to connect** (`/connect`, open to everyone) shows the server's address, ports and client date and offers a `clientinfo.xml` download with `PUBLIC_IP` and the server name from the Settings screen filled in. Once a client patch is published in [`patches/`](patches/README.md), players on the 2025-07-16 client get the client and [llchrisll's English translation](https://github.com/llchrisll/ROenglishRE) (its LANG tab, in the server's mode) from [WARPGATE](https://warp.legacygamers.net/), and a download of the [rpatchur](https://github.com/L1nkZ/rpatchur) patcher set up for this server, which installs the server's patched exe and custom files. Until then the page walks players through patching the client themselves with [WARP0716](https://github.com/CrazyBebop/WARP0716).
- **Player pages** (open to everyone, linked from the top bar): **Online** lists the characters in the game with their class, levels and map; **Rankings** lists the top 50 characters by base level (then experience) or by zeny, for every class or one, leaving out admins' characters; **News** shows `overrides/portal/news.md`, a Markdown file written by the desktop app's News screen or by hand. Each `## ` heading starts a post, newest first, and anything above the first one (such as the rules) stays at the top of the page. The patcher's window shows the newest post.
- **Items and Monsters** (`/items`, `/monsters`, open to everyone): search items by name, number or type, and monsters by name or number. An item's page shows its stats, price, effect script and the monsters that drop it; a monster's page shows its stats, drops and the maps it spawns on, with MVP respawn times. Everything comes from what the map server loads in the server's mode, including db/import, mods and custom items, with drop chances and exp at the server's rates. The map server's container refills the tables with `docker/rathena/lookup_db.py` each time the map server starts, when what it reads has changed ([decisions](docs/decisions/item-monster-lookup.md)).
- **Client patcher** (`/patcher/`): the patcher's window, `plist.txt` and the published `.thor` patches from `patches/`. The portal image cross-compiles rpatchur for Windows from a pinned commit, so the first portal build takes a few minutes longer. It also builds our own patcher (`portal/cmd/patcher`, [decisions](docs/decisions/own-patcher.md)), which will replace rpatchur once it's been confirmed on players' PCs; until then the How to connect page offers it as "the new patcher" beside rpatchur. Ours reads `/patcher/info.json` (server name, newest news post, its own checksum, so it updates itself) instead of the `/patcher/` page. The portal signs that info, which lists each patch's checksum, with a key made from `PATCH_SIGNING_KEY` in `.env`, so our patcher only applies patches and updates the server published ([decisions](docs/decisions/signed-patches.md)).
- Passwords are stored as `MD5(password)`, matching `use_MD5_passwords: yes`. Portal sessions live in memory, so restarting the portal logs everyone out of it (not out of the game).

The portal is a single Go program in `portal/`, with its pages built in. To work on it without Docker, point it at a database and run it:

```sh
cd portal
DB_HOST=127.0.0.1 DB_PASSWORD=... go run .   # listens on :8080 (PORTAL_ADDR)
go test ./...
```
