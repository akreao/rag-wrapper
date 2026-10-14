# Desktop app review

Questions asked about the finished desktop app (PR #3), one at a time.

- **The admin password is cleared from `.env` after the first start.** Setup writes `ADMIN_PASSWORD` so `db-init` can create the admin; once `db-init` has finished, the app blanks it so no plain-text copy stays on disk. `ADMIN_USER` stays. The admin changes their password in the portal like any player. If the database volume is deleted, put a password back in `ADMIN_PASSWORD` before the next start to recreate the admin.
- **The server runs only while the app is open.** Closing the window cancels any build or start in progress, runs `docker compose down` (the database volume is kept) and exits, showing the shutdown while it runs. Players are disconnected when the app closes, and the server doesn't come back by itself after a reboot.
- **A Settings panel sets the client date and renewal mode.** `PACKETVER` and pre-renewal/renewal are edited in the app instead of by hand in `.env`; saving a change rebuilds the rAthena image. Rates and other gameplay settings came later, with the Settings screen ([server-settings.md](server-settings.md)).
- **The LAN address is re-detected on every Start.** "Detect automatically" stays the default, so the server never advertises a stale address after the PC's IP changes; players still need the new address in `clientinfo.xml`, and a DHCP reservation on the router avoids that. A fixed address can be picked in the app's Connecting panel.
