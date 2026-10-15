# Publish client patch

Roadmap feature 10's last piece, in the desktop app: the "Publish client patch" screen that builds patches for the portal's patcher ([client-patcher.md](client-patcher.md)). PR #15.

- **One client-files folder, laid out like the client.** Steven puts the files players need in `~/.local/share/app.ragwrapper.desktop/client-files/`, as they sit in a game client folder. Only what may be handed out goes there: the patched exe, custom content and his own translation additions, never the full client or llchrisll's translation.
- **`data/` goes into `rag-wrapper.grf`; everything else into the client folder.** GRF content is packed into our own GRF rather than loose files, so nothing in the official `data.grf` or a player's loose `data/` folder is overwritten. `System/` files (lub/lua) have to be loose, since the client only reads them from the folder.
- **The patch writes `DATA.INI`.** This settles the question left open by the patcher hosting work. Once anything has gone into the GRF, every patch also writes `DATA.INI` as `0=rag-wrapper.grf`, `1=data.grf`. llchrisll's translation ships its own (`0=server.grf`, `1=data.grf`, in `Translation/Renewal`, which the pre-renewal install copies too), and players install it before the patcher, so the patch has to replace it; `server.grf` is only a placeholder name there, with no file behind it. Rewriting it in every patch puts it back if a player reinstalls the translation later.
- **Each publish carries only what changed.** The app keeps each published file's checksum in `patches/published.json` and builds THOR patches of the new, changed and removed files: `NNNN_data.thor` for the GRF and `NNNN_client.thor` for the client folder, numbered after the highest number already in `plist.txt`. The screen lists the changes before publishing. A file deleted from the folder is deleted from the players' clients.
- **The patches are written by the app itself.** rpatchur's `mkpatch` has no releases and would mean another Rust build; the THOR format is small, so the app writes it the way rpatchur's own writer (`gruf`) does, with the `data.integrity` checksums the patcher checks.
- **Publishing waits for files that can't be handed out as they are**: no `rag-wrapper.exe` (the portal switches players to the patcher after the first patch, and its Play button starts that exe, so the first patch must carry it), another `.exe`, a `.grf` or `DATA.INI` in the folder, or a name the client can't read. Names outside `data/` must be ASCII. Korean names under `data/` are written in the client's code page (CP949), whether they're spelled in Hangul or in the Latin letters GRF tools on Windows show.
- **Works on Windows** (reported 2026-10-15). A tester on a Windows PC ran the patcher with a published patch and the client works. Steven plays under Wine on Linux without serious issues; a Windows VM couldn't start the client at all (no real 3D), so Windows testing happens on a real PC.

Questions asked about the finished button (PR #15), one at a time.

- **Every patch overwrites `DATA.INI`.** Once a GRF has been published, each patch rewrites it with `rag-wrapper.grf` above `data.grf`, replacing llchrisll's copy and putting it back if a player reinstalls the translation later. Players never edit it by hand.
- **Files deleted from the folder are deleted from the players' clients.** Players' clients stay a copy of the client-files folder, so a dropped or renamed file doesn't linger. The screen lists each removal before publishing.
- **The client-files folder stays in the app's data folder**, `~/.local/share/app.ragwrapper.desktop/client-files/`, beside the backups. The screen's "Open client files folder" button opens it.

## Files for one game mode

Translation additions differ between Pre-Renewal and Renewal, and with separate characters per mode (#30) the server can switch (Steven, 2026-10-14).

- **Players follow the server.** The client-files folder gets `Pre-Renewal/` and `Renewal/` folders, laid out like the client folder, for files that differ by mode. A patch carries the shared files plus the server's current mode's folder (from `PRERE`); a mode file replaces a shared file of the same path. Chosen over a patch list per mode, because each patcher remembers how far through one list it is: switching lists would make players download a fresh patcher.
- **A mode switch is a publish.** After a switch, the Client patch screen says the server changed mode and lists the files that differ; publishing swaps them on every player's client, including removing the other mode's files. Until then players keep the old mode's files. `published.json` records the mode of the latest patch.
- **We still never hand out llchrisll's translation.** Players install the matching one from WARPGATE's LANG tab ([connect-page.md](connect-page.md)); these folders are only for the server owner's own additions.
