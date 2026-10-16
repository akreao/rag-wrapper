# Server updates

Roadmap feature 5, built in the desktop app. Each app version is released with one rAthena commit, the `RATHENA_COMMIT` in its bundled `.env.example`; the user's server keeps the commit in its own `.env` until they choose to move.

- **The app asks first.** When the two commits differ, a "Server update available" bar shows above every screen, with both short commit ids and a "What's changed" link to GitHub's comparison. Nothing happens until **Update server** is clicked; until then the old image keeps running.
- **Updating is backup, build, pin.** The database is backed up first (an automatic backup, "Before a server update"), then the images for the new commit are downloaded, or built where none is published ([prebuilt-images.md](prebuilt-images.md)). `.env` only gets the new commit once that build has succeeded, so a failed or cancelled build changes nothing and the update stays on offer.
- **The restart waits for a click.** A running server stays on the old version after the build; a "Restart on the new build" bar shows until the game servers run the new image. Clicking it runs a full `up`, so `db-init` applies the database upgrades before the game servers start. A stopped server picks the update up at its next Start. The same bar shows after a plain Rebuild, and "Restart game servers" on the Settings screen also does the full `up` while the image is newer, so new server code never starts against a database without its upgrades.
- **Each app version's commit is offered once.** `.env` remembers the last commit offered (`RATHENA_COMMIT_OFFERED`), set when updating or when "Not this one" is clicked. A commit set by hand afterwards, for example to test a fork change before releasing it, isn't offered back; the next app version's commit is. A new install starts on the app's commit and isn't offered it.
- **Players can play during the build.** Building doesn't touch the running containers; only the restart onto the new version disconnects players.

