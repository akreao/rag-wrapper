# How to connect page

Questions asked about the portal's "How to connect" page (roadmap feature 3, PR #6), one at a time.

- **Logged-out visitors land on How to connect.** Opening the portal's address without being logged in goes to `/connect`, the setup steps, instead of the login page; logged-in players still go to their account. Someone who isn't logged in is most likely new, and the steps start with creating an account.
- **The steps lead with the 2025-07-16 client.** Most players get that client from [WARPGATE](https://warp.legacygamers.net/), which downloads an unpatched client and keeps it updated without touching `clientinfo.xml`. When the server is built for `20250716` the page sends players to WARPGATE's RAGEXE tab for the clientinfo-ready exe and to [WARP0716](https://github.com/CrazyBebop/WARP0716) (Select Recommended) to patch it, because WARPGATE's own WARP tab can't download WARP0716 (it has no GitHub releases). Details are in the project's `client/patching-20250716.md`; the steps are unverified on Windows until the first real test. The page still follows whatever `PACKETVER` the server is built with; the `clientinfo.xml` it offers is the same for any client date.
- **The server's name in the client is a setting.** It's the same name the desktop app's Settings screen sets (rAthena's `server_name`, default "rAthena", in `overrides/char_conf.txt`). The portal reads that file, mounted read-only, and writes the name into `clientinfo.xml` and onto the page, so players see one name everywhere.

## English translation through WARPGATE

Both versions of the 2025-07-16 steps (the patcher, and patching by hand with WARP0716) send players to WARPGATE's **LANG** tab, choosing **Pre-Renewal**, instead of copying llchrisll's files by hand. Players already use WARPGATE for the client, and it installs and updates the translation for them.

- **Still not distributed by us.** WARPGATE installs [llchrisll's translation](https://github.com/llchrisll/ROenglishRE) from llchrisll's project; the portal only links to it and never hands it out.
- **`DATA.INI` still comes from our patches.** Whatever the translation install leaves in `DATA.INI`, the patcher's patches rewrite it with `rag-wrapper.grf` above `data.grf` ([publish-client-patch.md](publish-client-patch.md)). WARPGATE's page says it won't overwrite `DATA.INI` or `clientinfo.xml` on updates, so the patched copy stays.
- **The page follows the server's mode** (Steven, 2026-10-14). The portal gets `PRERE` like the rAthena build, shows the mode under "This server" and names it in the LANG step: Pre-Renewal (the default) or Renewal.
- **Verified vs inferred (2026-10-14):** the LANG tab, the Renewal/Pre-Renewal choice, automatic translation updates and leaving `DATA.INI` alone come from WARPGATE's own site. The exact install button label isn't on the site, so the page doesn't name it. Nothing has been tried on Windows yet.

## Only `clientinfo.xml`

The portal no longer offers `sclientinfo.xml`, on the connect page, as a download or in the patcher zip (Steven, 2026-10-14). It was the same file under the name Sakray (kRO's test server) clients read, and nobody plays on a Sakray client. A copy left in a player's `data` folder from an earlier download is harmless.
