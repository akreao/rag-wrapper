# rag-wrapper

A Linux desktop app that runs a private rAthena server and its database, for use with the real Ragnarok Online client.

## Desktop app

The app in [`desktop/`](desktop/README.md) runs the stack for you: first-run admin setup, start and stop, build progress, and each service's status and logs.

## Running the server stack

The server side runs in Docker: MariaDB, rAthena's login, char, map and web servers built from source for your client's packet version, and the account portal.

```sh
cp .env.example .env   # then change the passwords
docker compose up -d --build
docker compose logs -f login char map
```

The first build compiles rAthena and takes several minutes. On first start, `db-init` creates the database and imports rAthena's schema; later starts skip the import.

| Port | Server |
|------|--------|
| 6900 | login (point your client's `clientinfo.xml` here) |
| 6121 | char |
| 5121 | map |
| 8888 | web (emblems and settings for newer clients) |
| 8080 | account portal (`PORTAL_PORT`) |

Settings in `.env`:
- `PUBLIC_IP` is the address the client is sent for the char and map servers. Keep `127.0.0.1` when the client runs on this machine; use the machine's LAN IP for other computers.
- `PACKETVER` must match your client's date. Changing it needs a rebuild (`docker compose build`).
- `RATHENA_COMMIT` is the rAthena commit to build, from the `rag-wrapper` branch of `akreao/rathena`. Change it to pick up server changes, then rebuild. `RATHENA_REPO` points at a different fork.

Gameplay settings (rates, server name, message of the day and anything else in rAthena's configs) go in [`overrides/`](overrides/README.md) as rAthena `conf/import`-style files; restart the game servers (`docker compose up -d --force-recreate login char map`) to apply them. The desktop app's Settings screen writes the common ones for you.

`docker compose down` stops everything; the database lives in the `db-data` volume and survives restarts. `docker compose down -v` deletes it.

## Account portal

Players create and manage their accounts at `http://<server's LAN IP>:8080`. The game client's own `_M`/`_F` sign-up is off, so this is the way in.

- **Anyone on the LAN** can register (username, password, email, birthdate, sex), then log in to change their password, email and birthdate and see their characters. The game asks for the birthdate when deleting a character.
- **Admins** (group 99, such as the first admin created on setup) also get an Admin page: search accounts, change an account's group (GM level), ban for a set time or until lifted, and set a new password for a player who forgot theirs. Changes apply at the player's next game login; a player already in game stays on until they log out.
- Passwords are stored as `MD5(password)`, matching `use_MD5_passwords: yes`. Portal sessions live in memory, so restarting the portal logs everyone out of it (not out of the game).

The portal is a single Go program in `portal/`, with its pages built in. To work on it without Docker, point it at a database and run it:

```sh
cd portal
DB_HOST=127.0.0.1 DB_PASSWORD=... go run .   # listens on :8080 (PORTAL_ADDR)
go test ./...
```
