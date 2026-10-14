# Desktop app

A Tauri app that runs the server: it drives `docker compose` on the stack in this repository. It has no account features beyond creating the first admin; players use the web portal.

What it does:
- **First run** asks for an admin username and password and the address players connect to, then writes `.env`: the admin as `ADMIN_USER`/`ADMIN_PASSWORD`, `PUBLIC_IP`, and random `DB_PASSWORD`, `DB_ROOT_PASSWORD` and `INTER_PASSWORD`. `db-init` creates the admin as a GM on the next start, after which the app blanks `ADMIN_PASSWORD` so no plain-text copy stays on disk.
- **Start** runs `docker compose up --detach`, which builds the rAthena image first if it's missing. The build's output streams into the Activity pane with a step counter.
- **Stop** runs `docker compose down`. The database volume is kept.
- **Rebuild** runs `docker compose build`.
- **Server update available** shows above every screen when this version of the app comes with a different rAthena commit (`RATHENA_REPO` and `RATHENA_COMMIT` in the bundled `.env.example`) from the one in the user's `.env`, with a link to GitHub's list of the commits in between. Nothing changes until **Update server** is clicked. It backs up the database ("Before a server update"), builds the images at the new commit (passed to `docker compose build` in the environment, so `.env` still names the old one), and only once the build has succeeded writes the new commit into `.env`. If the server was running, it then runs `docker compose up --detach`: `db-init` applies the new commit's database upgrades and the game servers restart on the new image. A stopped server picks it up at its next Start. A failed or cancelled build leaves the old image running, `.env` untouched and the update on offer.
- **Settings** (the header button) has two parts:
  - **Game settings**: base and job exp rates, drop rates for common and usable items, equipment, cards and MVP rewards, the exp lost on death, the server name shown in the client, the message of the day, and whether players get rAthena's "Super Player" helper commands (@autoloot, @go, @rates, @whodrops and the rest). Saving writes rAthena overrides into the stack's `overrides/` folder (see below); they apply when the game servers next start, and the screen offers **Restart game servers**, which recreates the login, char and map containers while the database and portal keep running. Values left at official rates are left out of the files, so rAthena's stock config applies. **Open override folder** opens it for everything else.
  - **Server build**: the client date (`PACKETVER`, with 20250716, the default, and 20220406 as presets or any other date) and Pre-renewal (the default) or Renewal (`PRERE`). A change shows a warning, and saving it rebuilds the images and, if the server was running, starts it again on the new build.
- **Services** shows each service's state and published ports, refreshed every 3 seconds. Clicking one streams its logs.
- **Dashboard** (the header button opens its own page) lists the characters in game (name, class, base and job level, map) from the `char` table's `online` flag, read with the database's own `ragnarok` account in the `db` container, with the player count and how long the map server has been up. Below it, each running container's CPU (as a share of the whole computer) and memory from `docker stats`, with a total. It refreshes every 5 seconds while the page is open and the window isn't minimised. The list is empty while the char server is down, since the flags are only kept current while it runs.
- **Address players connect to** defaults to *Detect automatically*: on every Start the app writes the machine's current LAN IP (the interface with the default route, skipping Docker's bridges) as `PUBLIC_IP`. Picking an address pins it instead (`PUBLIC_IP_AUTO=no` in `.env`).
- While the `portal` service runs, a link opens the account portal.
- **Backups** (the header button opens its own screen) keeps gzipped dumps of the `ragnarok` database in `~/.local/share/app.ragwrapper.desktop/backups/` (`backups/` in the stack directory when `RAG_WRAPPER_STACK_DIR` is set). **Back up now** makes one; **Rebuild**, **Update server** and saving a build setting make one before the build, since the new build may upgrade the database on its next start. **Stop** and closing the app make one too, when there's no backup of any kind from the last 24 hours and the database is running. The newest 10 made with Back up now are kept, and separately the newest 10 made automatically, so rebuilds never push out a backup made by hand. A backup is `mariadb-dump --lock-all-tables` run in the `db` container (most of rAthena's tables are MyISAM, so a transaction alone wouldn't give a consistent copy; writes pause for the seconds it takes). It first repairs any table an unclean shutdown left marked as crashed, which MariaDB can't do under the dump's lock. If the server is stopped, the app starts the database for the backup and stops it again. Each file is named `ragnarok-YYYYMMDD-HHMMSS-<reason>.sql.gz`.
- **Restore** on a backup asks for confirmation, checks the file is a whole dump, stops every service but the database, backs up the current database ("Before a restore"), drops and recreates `ragnarok`, loads the dump, and starts the server again if it was running. `db-init` then applies any upgrade files the backup predates. **Open folder** opens the backups folder so files can be copied elsewhere; a file copied back in with the same naming shows up in the list.

The server runs only while the app is open: closing the window cancels any build or start in progress, runs `docker compose down` (keeping the database volume) and then exits.

## Where the stack lives

The app bundles `compose.yaml`, `docker/`, `portal/` and `.env.example`, and copies them to `~/.local/share/app.ragwrapper.desktop/stack/` on every launch, so app updates bring stack updates. `.env` there is the user's and is never overwritten. The Compose project is always named `rag-wrapper`, so containers and the `rag-wrapper_db-data` volume keep their names.

To point the app at another stack directory, such as this checkout while developing, set `RAG_WRAPPER_STACK_DIR`:

```sh
RAG_WRAPPER_STACK_DIR=$PWD/.. cargo run
```

## Overrides

`overrides/` in the stack directory is mounted into every rAthena container as `conf/user/`. On start, `entrypoint.sh` uses each file there that is named like one of rAthena's `conf/import/` files (`battle_conf.txt`, `char_conf.txt`, `map_conf.txt`, `groups.yml`, ...) in its place, before the stack's own values, so the database, addresses and server passwords can't be overridden by mistake. The Settings screen edits only its own lines in those files; see [`overrides/README.md`](../overrides/README.md).

## Building

Needs Rust, the Tauri prerequisites for Linux and Docker with the Compose plugin; the user running the app must be in the `docker` group.

```sh
# Ubuntu 24.04
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libxdo-dev libssl-dev build-essential file
cargo install tauri-cli --version "^2" --locked

cd desktop/src-tauri
cargo run                          # development build
cargo test                         # unit tests
cargo tauri build --bundles deb    # or appimage
```

The UI is plain HTML, CSS and JavaScript in `ui/`; there is no npm step.
