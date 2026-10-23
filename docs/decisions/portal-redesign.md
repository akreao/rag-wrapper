# Portal redesign

A new look for the web portal, with Ragnarok Online themes, and a home page. Built in the PR that adds this file.

- **The "game window" look (Steven, 2026-10-23).** Of three mockups (parchment and gold, game window, dark guild hall), Steven picked the one whose panels look like the game client's own windows: a blue title bar over a white body, on a sky-blue background. It reads as Ragnarok to anyone who has played, and stays readable on long pages like How to connect.
- **No Gravity art.** The logo and favicon are a pink blob drawn for the portal in the spirit of a Poring, not the game's sprite. Nothing from the client's GRFs is served. The footer names Ragnarok Online as Gravity's trademark.
- **Panels are the building block.** A page is a set of `<section class="panel">` blocks; the first heading in one becomes its title bar, and a table inside runs edge to edge under it. `.tiles` lays panels side by side and wraps them on narrow screens. The older `.card` and `.grid` classes are kept as the same thing, so a page that still uses them looks right.
- **All colours are tokens in `style.css`,** and a dark theme (night-sky background, dark panels) follows the browser's setting until a player picks one with the switch in the top bar (see Review).
- **The font is bundled.** Nunito (SIL Open Font License, licence in `static/fonts/OFL.txt`) is served by the portal itself, since a LAN server may have no internet. Browsers fall back to the system font if it fails to load.
- **A home page at `/`.** It used to send visitors to How to connect and players to their account. Now it shows a welcome with how many are playing, the mode, address and client date, the steps to start, the newest news post, who's online, the top five by level and a search box for the item, monster and market pages. If the database doesn't answer, the page still shows, with a note in place of the two lists.
- **Top bar: Play, News, Online, Rankings, Market, Database.** "Play" is How to connect; "Database" is the item and monster lookup. The current page is highlighted. Admin moved from the account links into the bar, shown to admins only. The server's name (from the desktop app's Settings) replaces "Ragnarok accounts".
- **The patcher window is unchanged.** It runs in Windows' old IE engine, which can't use this stylesheet, so `patcher.css` stays as it is.

## Review

Questions asked about the finished redesign (PR #93), one at a time.

- **Logging in lands on the home page (Steven, 2026-10-23).** It has the news and who's online, which players look at most; the account page is one click away in the top bar. Creating an account still lands on the account page, with its welcome message.
- **A light/dark switch in the top bar (Steven, 2026-10-23).** A sun or moon button overrides the system setting; the choice is kept in that browser's localStorage, so it's per browser, not per account. `static/theme.js` applies it before the page draws, so there's no flash. Without JavaScript the button stays hidden and the system setting applies.
- **The patcher window keeps its old look for now (Steven, 2026-10-23).** The launcher itself is due for a rework, so its window gets restyled then rather than twice.
- **Static files are revalidated, not cached blind (2026-10-23).** Each file under `/static/` gets an ETag, a hash of its contents worked out at start-up, with `Cache-Control: no-cache`. A browser keeps the font, `theme.js` and `style.css` and only checks the ETag on each page, getting a bodiless 304, so a portal update shows at once. A long max-age would need versioned URLs to avoid stale CSS after an upgrade, which is more moving parts for a LAN site.
