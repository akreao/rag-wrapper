# Decisions

Choices made while planning rag-wrapper, newest last. Each says what was decided and what it means for the build.

## Scope

- **Fresh app**, not built on Ragnarok Offline.
- **Linux only** for now. Docker is an accepted requirement.
- **Real game client**, no roBrowser. `PACKETVER` is 20220406 until the client's date is confirmed.
- rAthena is built from `akreao/rathena` `master`.

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

## CI: GitHub Actions

Questions asked about the CI workflow (PR #5), one at a time.

`.github/workflows/ci.yml` runs on every PR and every push to `main`: the portal's `gofmt`, `go vet` and `go test`; `docker compose config` and `shellcheck` on the entrypoint; and a build of the rAthena image, only when `docker/rathena/`, `.env.example`, `compose.yaml` or the workflow changes.

- **CI must pass before a PR merges to `main`.** The checks are required in the branch settings, so a broken portal or a bad rAthena pin can't land by accident. A skipped rAthena image job counts as passed.
