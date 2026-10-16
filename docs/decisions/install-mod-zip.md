# Install a mod from a zip

A button on the desktop app's Mods screen, **Install from zip…**, that unpacks a mod someone sent as a zip into the mods folder, so nobody has to unzip it and find the mods folder by hand. Builds on the [mods folder](mods-folder.md).

- **The zip becomes a new folder in `mods/`, switched off**, like the item kit's New item mod. The person switches it on and restarts when they're ready, so nothing loads (and no database backup runs) the moment it's installed.
- **The mod's name comes from the zip.** When everything in the zip is inside one folder (someone zipped the mod's folder), that folder's name is the mod's name and the folder itself isn't repeated inside. Otherwise (the zip holds `npc/`, `db/` and `client/` directly) the zip's file name is. Characters a mod's name can't have become `-`. macOS's `__MACOSX/` and `.DS_Store` files, and Windows' `Thumbs.db`, are left out.
- **It has to look like a mod**: at least one of `npc/`, `db/` or `client/` at the top (inside the single folder, if there is one). Anything else is refused with a message saying what a mod's zip needs.
- **An existing mod is never replaced.** When the mods folder already has a folder with that name, the install stops and says to remove or rename it first, so a hand-edited mod can't be overwritten by accident.
- **Nothing is written outside the new folder.** A zip is untrusted, so:
  - an entry whose path could leave the folder (`..`, an absolute path, a drive letter like `C:`) stops the whole install, since only a broken or hostile zip has one;
  - links in the zip are left out (and listed), as the client patch already refuses links out of a mod;
  - backslashes count as folder separators, since some Windows tools write them;
  - it unpacks into a hidden `mods/.install-<name>` folder that the Mods screen ignores, and is renamed to the mod's name only when every file is out; a failed install removes it;
  - the unpacked size is capped at 4 GB, counted from what actually comes out rather than what the zip claims, and the zip at 100,000 entries.
- **Two files whose names differ only in capitals keep the first**, with a note, since the players' Windows clients can't hold both.
- **The zip goes from the screen to the app as raw bytes** (Tauri's raw request body, with the file name in a header), from an ordinary file picker. No dialog plugin or new permission was needed.
