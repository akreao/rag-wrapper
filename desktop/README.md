# Desktop app

A Tauri app that runs the server: it drives `docker compose` on the stack in this repository. It has no account features beyond creating the first admin; players use the web portal.

What it does:
- **First run** asks for an admin username and password and the address players connect to, then writes `.env`: the admin as `ADMIN_USER`/`ADMIN_PASSWORD`, `PUBLIC_IP`, and random `DB_PASSWORD`, `DB_ROOT_PASSWORD` and `INTER_PASSWORD`. `db-init` creates the admin as a GM on the next start, after which the app blanks `ADMIN_PASSWORD` so no plain-text copy stays on disk.
- **Start** runs `docker compose up --detach`, which builds the rAthena image first if it's missing. The build's output streams into the Activity pane with a step counter.
- **Stop** runs `docker compose down`. The database volume is kept.
- **Rebuild** runs `docker compose build`.
- **Server build** sets the client date (`PACKETVER`) and Renewal or Pre-renewal (`PRERE`). Saving a change rebuilds the images and, if the server was running, starts it again on the new build.
- **Services** shows each service's state and published ports, refreshed every 3 seconds. Clicking one streams its logs.
- **Address players connect to** defaults to *Detect automatically*: on every Start the app writes the machine's current LAN IP (the interface with the default route, skipping Docker's bridges) as `PUBLIC_IP`. Picking an address pins it instead (`PUBLIC_IP_AUTO=no` in `.env`).
- While the `portal` service runs, a link opens the account portal.

The server runs only while the app is open: closing the window cancels any build or start in progress, runs `docker compose down` (keeping the database volume) and then exits.

## Where the stack lives

The app bundles `compose.yaml`, `docker/`, `portal/` and `.env.example`, and copies them to `~/.local/share/app.ragwrapper.desktop/stack/` on every launch, so app updates bring stack updates. `.env` there is the user's and is never overwritten. The Compose project is always named `rag-wrapper`, so containers and the `rag-wrapper_db-data` volume keep their names.

To point the app at another stack directory, such as this checkout while developing, set `RAG_WRAPPER_STACK_DIR`:

```sh
RAG_WRAPPER_STACK_DIR=$PWD/.. cargo run
```

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
