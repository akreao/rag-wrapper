# Desktop app UI redesign

The desktop app's new look (Steven, 2026-10-22: "more sleek, modern and modular, not so reliant on simple buttons"). Built in several PRs; the first is the app shell (sidebar, header, notices).

- **Direction A, "Sidebar", picked from three mockups** (sidebar, dark console, tiles). It scales to the app's many screens and clears the crowded header. The mockups also showed a Settings screen of grouped rows with switches and steppers, which goes with it.
- **A dark sidebar lists every screen, grouped**: Server (Overview, Players, Events), Content (Mods, Client patch, News) and System (Backups, Settings). It replaces the row of header buttons that turned into "Back to server". Clicking the screen that's open does nothing, so unsaved edits on it stay. The sidebar is hidden until first-run setup is done.
- **The old Dashboard screen is called Players**, since it's mostly who's online and messaging them; the main screen is **Overview**.
- **The header shows the screen's name, a status pill, and only the action that does something now**: Start server while the game servers aren't all up, Stop server while anything runs (both while it's partly up). Rebuild and Quit moved into a ⋯ menu, each with a line saying what it does.
- **Notices stay in view, as cards under the header.** A bell with their count tucks them away for now; a notice that wasn't showing before brings them back. They aren't moved into a closed popover, because several (a restart waiting, example passwords) need acting on.
- **Inter is bundled** (`desktop/ui/fonts`, SIL Open Font License, 120 KB), so the app looks the same whatever fonts the desktop has. Latin and Latin Extended only; anything else falls back to the system font.
- **Light and dark follow the desktop's setting**, as before; the sidebar is dark in both.
- **Screenshots for PRs come from the UI in Chromium with Tauri's API stubbed** (`/mnt/project-files/ui-redesign/preview/` in the project's files), since Steven tests only released AppImages. They show layout, not the real WebKitGTK rendering.
- **Overview opens with three cards**: status (with uptime and how many services run), players online (with a line of the last day's counts and the peak), and the address players connect to, with a copy button. Services become a grid of small tiles beside the Activity log, which keeps most of the screen, since a build's output and progress show there. **Online now** lists up to 8 characters; **All players** opens the Players screen.
- **The address picker, "This network only" and the portal port moved to a Network card in Settings.** They're set once and rarely changed, so they don't need space on the main screen; the address card's **Change** link opens them.
- **Overview reads who's online and the CPU and memory every 15 seconds**, a third as often as the Players screen, since `docker stats` isn't free and Overview is open most of the time. Neither runs while the window is hidden.
- **Settings is split into tabs** (Game, Server build, Network, Version, App), one card each, instead of one long page. Each option is a row: its name and explanation on the left, the control on the right. On/off options are switches. Game and Server build keep their own Save buttons, in a bar at the foot of the card, since they apply differently (on the next restart, or by rebuilding).
- **Rates stay in Battle config** rather than coming back as steppers on the Game tab, as the mockup showed. The rates' one home is rAthena's `conf/battle` files (battle-config.md); two places to set them would disagree.
- **Each screen's actions sit in a toolbar at the top**, the main one highlighted (Install from zip, Back up now, New post). Check again and Reload are icon buttons.
- **Long explanations fold away** under a "How mods work" style heading, closed by default, so the list a screen is about comes first. Errors and save notices stay outside them, where they can't be missed.
- **Mods and rAthena's custom NPCs are switched on and off with switches.**
- **Every element with the `hidden` attribute is hidden**, whatever its own `display`, which fixes the mods screen's file picker showing beside Install from zip.
- **Icon styles apply only to decorative icons** (`aria-hidden` SVGs). Part 1's rule for every SVG had outlined the Players chart's labels.
- **Restart server is in the ⋯ menu too** (Steven, review), above Rebuild, while the game servers are all up. It's the same restart as after saving game settings: players online get the 5-minute warning first. The tray keeps only Start, Stop and Quit.
