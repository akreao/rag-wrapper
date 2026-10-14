# Decisions

Choices made while planning rag-wrapper, newest last. Each says what was decided and what it means for the build.

## Scope

- **Fresh app**, not built on Ragnarok Offline.
- **Linux only** for now. Docker is an accepted requirement.
- **Real game client**, no roBrowser. `PACKETVER` defaults to 20250716, the client most people get through WARPGATE; 20220406 is the other common client and is offered as a preset in the app's Settings. Installs set up before this keep the value already in their `.env`.
- **Pre-renewal by default** (`PRERE=yes`). Pre-renewal is the more polished mode in rAthena, while its Renewal content is incomplete and not up to date with kRO. Renewal stays selectable in the app's Settings. Installs set up before this keep the mode already in their `.env`.
- rAthena is built from the `rag-wrapper` branch of `akreao/rathena` (see below).

## Who connects: LAN

The server is for this machine and others on the same local network, not the internet.

- The app detects the machine's LAN IP and uses it as `PUBLIC_IP`, so other PCs on the network are sent a reachable char and map address.
- Ports 6900, 6121, 5121 and 8888 are published on the LAN. No port forwarding, public IP or DNS setup.
- Moving to internet play later means changing `PUBLIC_IP` and tightening account defaults; nothing else in the stack has to change.

## rAthena image: built locally

The app builds the rAthena image on the user's machine with `docker compose build`, from `docker/rathena/Dockerfile`.

- First start, and any change to `PACKETVER` or renewal mode, triggers a rebuild of roughly 5 to 10 minutes. The app shows build progress and logs while it runs.
- No container registry or publishing CI is needed. Prebuilt images can be added once the client's `PACKETVER` is settled.

## Accounts: a slim web portal

Players register and manage their accounts in a small web server that runs as another service in the Compose stack, reachable on the LAN.

- Pages to work towards: register, log in, change password and email, and view the account's characters.
- It talks to the same `ragnarok` database rAthena uses; it doesn't depend on rAthena's own `web-server`, which stays for the client's emblem and settings features.
- The client's built-in `_M`/`_F` sign-up stays off, so the portal is the one way in.

## Desktop app: server control only

The desktop app's one job is running the server: start and stop the stack, show each service's status and logs, and show build progress. It has no account features.

- All account work, player self-service and admin (GM level, bans) alike, lives in the web portal, so it works from any PC on the LAN.
- The desktop app can link to the portal but doesn't embed it.

## Passwords: rAthena's MD5 option

`use_MD5_passwords: yes` in the login server config, and the portal stores `MD5(password)` the same way.

- Better than plain text, but unsalted MD5 is weak if the database is copied. Acceptable while the server is LAN-only.
- The inter-server account (account 1) must be stored as an MD5 hash too, so `entrypoint.sh` hashes `INTER_PASSWORD` when it writes it.
- Proper hashing (bcrypt) in the rathena fork is the follow-up if the server is ever opened to the internet.

## Registration: open to anyone on the LAN

Whoever can reach the portal can register; there's no invite code or approval step. This relies on the server being LAN-only. An invite code is the first thing to add if that changes.

## First admin: created in the desktop app's first-run setup

On the first start, the desktop app asks for an admin username and password. The setup step (`db-init`) creates that account with GM rights, stored as MD5 like every other password.

- This is the desktop app's only account-related job; everything after it happens in the portal.
- Nobody on the LAN can claim admin by registering first.

## Web portal: Go

A single Go binary serving plain server-rendered HTML, no JavaScript framework, in its own small image in the Compose stack.

## Desktop app: Tauri

Tauri, as proposed in the first plan and not changed since. It drives `docker compose` for start, stop, status, logs and builds, and runs the first-run admin setup.

## Web portal: details settled after the first build

Questions asked about the finished portal (PR #2), one at a time.

- **Forgotten passwords: an admin sets a new one.** There's no email on a LAN server, so the admin page for an account has a "Reset password" form. The admin tells the player the new password and the player changes it on their own account page. Email reset links are out.
- **Bans don't kick players already in game.** The portal only writes the database, so a ban stops the next login. The ban form tells admins to use `@kick` in game for someone who's online. No rathena fork change for this.
- **Birthdate is required at sign-up.** With `char_del_option: 2` the game checks the birthdate when a player deletes a character, so an account without one could never delete characters. Players can change it on their account page.
- **Admin pages are for Admins (group 99) only.** Lower GM groups such as Law Enforcement get no portal access. A moderator role (ban and reset, but no GM-level changes) can be added later if needed.

# Docker stack review

Questions asked after the Docker stack (PR #1) merged.

## rAthena source: a pinned commit on the fork's `rag-wrapper` branch

The image builds one exact commit, `RATHENA_COMMIT` in `.env`, from the `rag-wrapper` branch of `akreao/rathena`. That branch started from the fork's `master` and is where server changes for this app get committed.

- Builds are repeatable: the same `.env` always builds the same server.
- Picking up a server change means committing it to `rag-wrapper`, then moving `RATHENA_COMMIT` to that commit and rebuilding.
- The Dockerfile fetches the commit by SHA, so it must be a full 40-character SHA.

## Database upgrades: applied automatically

`db-init` applies rAthena's `sql-files/upgrades/upgrade_*.sql` files on start, tracked in a `wrapper_sql_upgrades` table.

- When the table is empty (a fresh import, or a database from before this existed), every current file is recorded as applied without running, because `main.sql` already includes them.
- After moving `RATHENA_COMMIT`, any new upgrade files run once each, in date order. A failure stops setup, so the servers don't start against a half-upgraded database.
- Files are tracked rather than re-run because some upstream upgrades aren't safe to run twice. Optional extras like `premium_storage.sql` are not applied.

## Start-up: only when the user starts the server

Containers use `restart: on-failure`: Docker restarts a server that crashes, but nothing starts on boot or when Docker starts. The desktop app is the on switch.

- A "start server when the computer starts" toggle in the app can come later; it would switch the policy to `unless-stopped`.

## Backups: in the desktop app

Already decided in the roadmap (`roadmap/future-features.md`, feature 2): a "Back up now" button, an automatic backup before every rebuild or rAthena update, the last 10 kept, and restore from a list with the servers stopped first.

- Nothing is added to `db-init` for this. Upgrades only run after a rebuild, and the app's backup runs before that rebuild, so a second dump in setup would duplicate it.

## Server passwords: generated by the desktop app

The desktop app (PR #3) generates `DB_PASSWORD`, `DB_ROOT_PASSWORD` and `INTER_PASSWORD` on first run and writes them to `.env`. Nobody types them. `.env.example` stays for running the stack by hand without the app.

# CI: GitHub Actions

Questions asked about the CI workflow (PR #5), one at a time.

`.github/workflows/ci.yml` runs on every PR and every push to `main`: the portal's `gofmt`, `go vet` and `go test`; `docker compose config` and `shellcheck` on the entrypoint; and a build of the rAthena image, only when `docker/rathena/`, `.env.example`, `compose.yaml` or the workflow changes.

- **CI must pass before a PR merges to `main`.** The checks are required in the branch settings, so a broken portal or a bad rAthena pin can't land by accident. A skipped rAthena image job counts as passed.
- **The desktop app's CI job runs compile checks only.** Each PR runs `cargo check` and `clippy`; the AppImage is built only when a release is tagged. The job arrived with the desktop app PR (#3) and also runs its unit tests.

# Desktop app review

Questions asked about the finished desktop app (PR #3), one at a time.

- **The admin password is cleared from `.env` after the first start.** Setup writes `ADMIN_PASSWORD` so `db-init` can create the admin; once `db-init` has finished, the app blanks it so no plain-text copy stays on disk. `ADMIN_USER` stays. The admin changes their password in the portal like any player. If the database volume is deleted, put a password back in `ADMIN_PASSWORD` before the next start to recreate the admin.
- **The server runs only while the app is open.** Closing the window cancels any build or start in progress, runs `docker compose down` (the database volume is kept) and exits, showing the shutdown while it runs. Players are disconnected when the app closes, and the server doesn't come back by itself after a reboot.
- **A Settings panel sets the client date and renewal mode.** `PACKETVER` and pre-renewal/renewal are edited in the app instead of by hand in `.env`; saving a change rebuilds the rAthena image. Rates and other gameplay settings came later, with the Settings screen (below).
- **The LAN address is re-detected on every Start.** "Detect automatically" stays the default, so the server never advertises a stale address after the PC's IP changes; players still need the new address in `clientinfo.xml`, and a DHCP reservation on the router avoids that. A fixed address can be picked in the app's Connecting panel.

# Server settings review

Questions asked about the finished Settings screen (PR #7), one at a time. The screen writes rAthena override files into the stack's `overrides/` folder (see `overrides/README.md`), which apply when the game servers restart.

- **Player commands give the full "Super Player" list.** Turning the setting on gives every player all of rAthena's Super Player commands, @go (free teleport to towns) and @autotrade included. The setting is off by default.
- **"Exp lost on death" is one value for both base and job exp.** Official is 1%; 0 turns the penalty off. A server that wants different values sets `death_penalty_base` and `death_penalty_job` by hand in `overrides/battle_conf.txt`; the screen shows the base value, and saving from it sets both to that again.
- **Saving never restarts the server by itself.** While the server runs, saving shows a "Restart game servers" button, so several changes can go out in one restart, at a time that suits the players online. A restart disconnects everyone in game; the database and portal keep running.
- **The message of the day is plain ASCII only.** Accented and non-Latin text shows up garbled unless it matches the client's language, so the screen accepts only English letters, digits and punctuation.

# How to connect page

Questions asked about the portal's "How to connect" page (roadmap feature 3, PR #6), one at a time.

- **Logged-out visitors land on How to connect.** Opening the portal's address without being logged in goes to `/connect`, the setup steps, instead of the login page; logged-in players still go to their account. Someone who isn't logged in is most likely new, and the steps start with creating an account.
- **The steps lead with the 2025-07-16 client.** Most players get that client from [WARPGATE](https://warp.legacygamers.net/), which downloads an unpatched client and keeps it updated without touching `clientinfo.xml`. When the server is built for `20250716` the page sends players to WARPGATE's RAGEXE tab for the clientinfo-ready exe and to [WARP0716](https://github.com/CrazyBebop/WARP0716) (Select Recommended) to patch it, because WARPGATE's own WARP tab can't download WARP0716 (it has no GitHub releases). Details are in the project's `client/patching-20250716.md`; the steps are unverified on Windows until the first real test. The page still follows whatever `PACKETVER` the server is built with; the `clientinfo.xml` it offers is the same for any client date.
- **The server's name in the client is a setting.** It's the same name the desktop app's Settings screen sets (rAthena's `server_name`, default "rAthena", in `overrides/char_conf.txt`). The portal reads that file, mounted read-only, and writes the name into `clientinfo.xml` and onto the page, so players see one name everywhere.

# Admin character fixes

Roadmap feature 7's last pieces, added to the portal's admin pages: account search by character name, moving a stuck character to its save point, and a read-only inventory and zeny view. Giving or removing items stays in game (`@item`, `@delitem`).

- **Moving a character only works while it's logged out.** The portal copies `save_map`/`save_x`/`save_y` into `last_map`/`last_x`/`last_y` in one update that also checks `online = 0`, so a player who logs in at the same moment keeps their position. The char server reads the position from the database when a character is picked, so the move applies at the next login. For a character in game, the page points to `@jumpto` or `@recall`.
- **Item names come from the server's own item database.** On every start `db-init` reads `db/pre-re` or `db/re` (from `PRERE`) plus `db/import/item_db.yml` and refills a `wrapper_item_names` table, which the portal reads. Names follow rAthena updates, the renewal mode and custom items with no copy in the portal. If the table is missing or the load fails, the portal shows item numbers and the servers start anyway.
- **The inventory of a character in game is as of its last save.** The map server saves every few minutes (`autosave_time`), so the page can be slightly behind; it says so.
