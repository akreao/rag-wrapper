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
