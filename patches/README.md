# Client patches

The account portal serves this folder to the players' client patcher ([rpatchur](https://github.com/L1nkZ/rpatchur), or our own replacement for it in `portal/cmd/patcher`, which reads these files the same way), which they download from the portal's "How to connect" page. The portal mounts it read-only; the desktop app's "Publish client patch" button writes it (see `desktop/README.md`).

- `plist.txt` lists the published patches in the order they apply, one per line: a number, a space and the file name, e.g. `1 0001_client.thor`. That's the format THOR patchers read. Numbers only go up; never renumber or reuse one, because each player's patcher remembers the last number it applied.
- Each patch is a THOR archive (`.thor`, built with rpatchur's `mkpatch`) in this folder. Names may use letters, digits, `.`, `_` and `-`. A `.thor` file that isn't in `plist.txt` isn't served, so write the file first and add its line last.
- The first patch carries the patched 2025-07-16 client exe as `rag-wrapper.exe` in the client folder (a `mkpatch` definition with `use_grf_merging: false`); the patcher's Play button starts that file.
- Later patches carry custom GRF content and translation additions. Patches with `use_grf_merging: true` go into `rag-wrapper.grf`, which the patched client only reads if the client's `DATA.INI` lists it above `data.grf`.
- The desktop app names its patches `NNNN_data.thor` (into `rag-wrapper.grf`) and `NNNN_client.thor` (into the client folder); one publish can write one of each, GRF first. Once anything has gone into the GRF, every `_client` patch also writes `DATA.INI` with `rag-wrapper.grf` above `data.grf`, which puts it back if a player's translation install (llchrisll's ships its own, listing `server.grf`) replaced it.
- `published.json` is the desktop app's record of what players have: each file's checksum, so the next patch only carries what changed, and the publish history. The portal doesn't serve it. Deleting it makes the next publish send every file again.
- Only what the server owner may hand out goes here: the patched exe, custom GRFs and the owner's own translation additions. Never the full client or llchrisll's English translation, which players install themselves.

With no `plist.txt`, the patcher reports the client as up to date, and the "How to connect" page shows the steps for patching the client by hand instead of the patcher.
