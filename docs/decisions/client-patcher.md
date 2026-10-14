# Client patcher in the portal

Roadmap feature 10's portal side: the portal hosts an [rpatchur](https://github.com/L1nkZ/rpatchur) patcher for players, and the "How to connect" page offers it. Publishing patches (the desktop app's "Publish client patch" button) is a separate piece of work.

- **What's distributed** (from the roadmap): only the patched 2025-07-16 client exe, custom GRFs and the server owner's own translation additions. Never the full client or llchrisll's English translation, which has no licence; the page links players to both instead.
- **rpatchur is built into the portal image.** It has no published releases, so the portal's Dockerfile cross-compiles it for Windows (`x86_64-pc-windows-gnu`) from a pinned commit (`RPATCHUR_COMMIT`, `21a5482`). That keeps "build the image locally"; the stage is cached, so only the first build is slower. CI builds the portal image when its Dockerfile changes and checks the exe is a Windows GUI program. A cross-compiled exe has rpatchur's default look but no icon.
- **One download sets up a client.** `/connect/rag-wrapper-patcher.zip` holds `rag-wrapper-patcher.exe`, its `rag-wrapper-patcher.yml` (rpatchur reads the yml named after the exe), `data/clientinfo.xml`, and rpatchur's licences. Players extract it into the client folder. The config and connection file are written for each download with the address the player reached the portal on, like the `clientinfo.xml` download.
- **Patches live in the stack's `patches/` folder** (`patches/README.md`), mounted read-only into the portal: `plist.txt` in the standard THOR format plus the `.thor` files. Only files listed in `plist.txt` are served, so a patch goes live when its line is added. The desktop app creates the folder so Docker doesn't make it as root.
- **Fixed names in the client folder.** The first patch installs the patched exe as `rag-wrapper.exe`, which the patcher's Play button starts; GRF patches go into `rag-wrapper.grf`. Neither overwrites an official file. The publish button has to follow these names, and the client's `DATA.INI` must list `rag-wrapper.grf` above `data.grf` (open for the publish button: whether a patch writes `DATA.INI`, given llchrisll's translation may ship its own).
- **The patcher's window is a portal page** (`/patcher/`): server name, progress, Play and Exit, and links to the account and How to connect pages that open in the browser. rpatchur shows it with Windows' old IE engine, so its CSS and script are plain ES5.
- **The connect page switches when there's something to install.** For a `20250716` server with at least one published patch, the steps are: account, client from WARPGATE, llchrisll's translation, download the patcher, start it and click Play. With nothing published yet, it keeps the hand-patching steps with WARP0716 (plus the translation link), since the patcher would have no exe to give. Other client dates keep the generic WARP steps.
- **Not verified on Windows yet.** The cross-compiled patcher, the page in rpatchur's window and the first patch get their first real test once the publish button can build a patch.

Questions asked about the finished patcher hosting (PR #12), one at a time.

- **The hand-patching steps stay until the first patch is published.** Until `plist.txt` lists a patch, the page shows the WARPGATE RAGEXE plus WARP0716 steps, so players can join before the "Publish client patch" button exists. Once a patch is published, the page switches to the patcher.
- **Anyone who can reach the portal can download the patcher.** Like the How to connect page and the `clientinfo.xml` downloads, the zip and the patches need no login, so new players can set up the client before they make an account. The portal is only reachable on the LAN or VPN, and rpatchur can't log in to fetch patches anyway.
- **Play closes the patcher.** Clicking Play starts `rag-wrapper.exe` and closes the patcher's window, as most RO patchers do. Players open the patcher again the next time they play, which also picks up new patches.

## Patcher window fix (Steven's first test)

Steven's first run of the patcher: it didn't check for patches on start, Try again left it on "Checking for updates...", and Exit did nothing.

- **Cause (read from the source, not seen on Windows).** The page only called rpatchur when `typeof window.external.invoke` wasn't `"undefined"`. rpatchur's IE host (web-view 0.7.3, `webview_mshtml.c` in webview-sys 0.6.2) answers a property read of `invoke` with an empty value, so `typeof` is `"undefined"` there even though calling it works. The page decided it wasn't in the patcher and never sent `start_update`, `exit` or `play`. rpatchur itself doesn't check for patches until the page asks.
- **Fix.** The page calls `window.external.invoke` directly and treats a thrown error as "not in the patcher", which is how a normal browser behaves. It asks for an update on load, as rpatchur's own examples do.
- **No silent hangs.** If the patcher says nothing for 30 seconds after a check, the page says so and shows Try again. A script error in the page shows in the window, since rpatchur's window has no developer tools.
- **Tested** in Chromium with a stand-in for rpatchur's `external.invoke`: the commands sent on load and by each button, the timeout and the plain-browser message. The fix still needs a run of the real patcher on Windows.
