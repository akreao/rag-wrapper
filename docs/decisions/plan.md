# The plan

Choices made while planning rag-wrapper, before the first build. Each says what was decided and what it means for the build.

## Scope

- **Fresh app**, not built on Ragnarok Offline.
- **Linux only** for now. Docker is an accepted requirement.
- **Real game client**, no roBrowser. `PACKETVER` defaults to 20250716, the client most people get through WARPGATE; 20220406 is the other common client and is offered as a preset in the app's Settings. Installs set up before this keep the value already in their `.env`.
- **Pre-renewal by default** (`PRERE=yes`). Pre-renewal is the more polished mode in rAthena, while its Renewal content is incomplete and not up to date with kRO. Renewal stays selectable in the app's Settings. Installs set up before this keep the mode already in their `.env`.
- rAthena is built from the `rag-wrapper` branch of `akreao/rathena` (see [docker-stack.md](docker-stack.md)).

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
