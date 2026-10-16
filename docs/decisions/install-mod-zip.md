# Install a mod from a zip

A button on the desktop app's Mods screen, **Install from zip…**, that unpacks a mod someone sent as a zip into the mods folder, so nobody has to unzip it and find the mods folder by hand. Builds on the [mods folder](mods-folder.md). PR #83.

- **The zip becomes a new folder in `mods/`, switched off**, like the item kit's New item mod. The person switches it on and restarts when they're ready, so nothing loads (and no database backup runs) the moment it's installed.
- **The mod's name comes from the zip.** When everything in the zip is inside one folder (someone zipped the mod's folder), that folder's name is the mod's name and the folder itself isn't repeated inside. Otherwise (the zip holds `npc/`, `db/` and `client/` directly) the zip's file name is. Loose files beside that one folder, like a `README.txt` or a licence, don't count against it: they're left out, and listed. Characters a mod's name can't have become `-`. macOS's `__MACOSX/` and `.DS_Store` files, and Windows' `Thumbs.db`, are left out.
- **It has to look like a mod**: at least one of `npc/`, `db/` or `client/` at the top (inside the single folder, if there is one). Anything else is refused with a message saying what a mod's zip needs.
- **Installing a mod that's already there asks before replacing it**, since updating a mod is the usual reason. On Replace, the zip unpacks first, then the old folder moves aside, the new one takes its name and the old one is deleted (a link is removed, not followed). The mod's line in `enabled.txt` is untouched, so it stays on or off in the same load-order place. Cancel leaves everything as it was.
- **Nothing is written outside the new folder.** A zip is untrusted, so:
  - an entry whose path could leave the folder (`..`, an absolute path, a drive letter like `C:`) stops the whole install, since only a broken or hostile zip has one;
  - links in the zip are left out (and listed), as the client patch already refuses links out of a mod;
  - backslashes count as folder separators, since some Windows tools write them;
  - it unpacks into a hidden `mods/.install-<name>` folder that the Mods screen ignores, and is renamed to the mod's name only when every file is out; a failed install removes it;
  - the unpacked size is capped at 4 GB, counted from what actually comes out rather than what the zip claims, and the zip at 100,000 entries;
  - the zip itself is capped at 1 GB, checked by the screen before it reads the file and again by the app, since the whole zip is held in memory (in the window, then in the app). A bigger one gets a message to unzip it and copy the mod's folder into the mods folder instead.
- **Two files whose names differ only in capitals keep the first**, with a note, since the players' Windows clients can't hold both.
- **The zip goes from the screen to the app as raw bytes** (Tauri's raw request body, with the file name in a header), from an ordinary file picker. No dialog plugin or new permission was needed.

Questions asked about the finished feature (PR #83), one at a time.

- **Replace an installed mod when its zip is installed again?** Ask, then replace (Steven, 2026-10-16), as above: updating a mod is the usual reason to install the same zip again, and the confirm step guards against accidents.
