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
