# rag-wrapper

A Linux desktop app that runs a private rAthena server and its database, for use with the real Ragnarok Online client.

## Desktop app

The app in [`desktop/`](desktop/README.md) runs the stack for you: first-run admin setup, start and stop, build progress, and each service's status and logs.

## Running the server stack

The server side runs in Docker: MariaDB plus rAthena's login, char, map and web servers, built from source for your client's packet version.

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

Settings in `.env`:
- `PUBLIC_IP` is the address the client is sent for the char and map servers. Keep `127.0.0.1` when the client runs on this machine; use the machine's LAN IP for other computers.
- `PACKETVER` must match your client's date. Changing it needs a rebuild (`docker compose build`).
- `RATHENA_REPO` and `RATHENA_REF` choose the rAthena source to build.

`docker compose down` stops everything; the database lives in the `db-data` volume and survives restarts. `docker compose down -v` deletes it.
