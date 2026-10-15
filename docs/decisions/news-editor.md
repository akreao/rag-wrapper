# News editor

The desktop app's News screen, which writes the account portal's news file, `overrides/portal/news.md` (the follow-up to roadmap feature 18, see `player-pages.md`). Built in the PR that adds this file.

- **A screen in the main window, beside Mods**, not a window of its own like the battle config editor. It's a short list of posts, and the other screens with lists (Backups, Client patch, Mods) are main-window screens too.
- **One card per post, newest on top**: a title and the post's Markdown text. **New post** adds an empty card on top. The first card says "Shown in the patcher", since the patcher's window shows only the newest post.
- **Anything above the first post is kept and editable** in a folded "Top of the News page" box, so a `# Rules` section written by hand survives the editor.
- **One Save for the whole file**, rather than one per post. Delete and New post take effect on Save, so Reload undoes them; there's no "are you sure?" dialog. Unsaved changes stay while other screens are open, but closing the app drops them.
- **The editor writes the file in the portal's layout**: the top part, then each post as `## Title`, a blank line and its text, posts separated by a blank line. Splitting the file follows the portal's `latestPost` rule, so `## ` lines inside ``` or ~~~ code blocks don't start a post. A file already in this layout reads back byte for byte, so the editor doesn't reshuffle it. Deleting every post and emptying the top removes the file, which the portal shows as "No news yet."
- **No restart and no portal change are needed.** The portal opens `news.md` on every request to the News page and to `/patcher/info.json`, and `overrides/` is a folder mount, so a new or replaced file shows straight away. The app writes a temporary file and renames it over `news.md`, so the portal never reads half a post.
- **A hand edit isn't overwritten.** The screen remembers the file as it loaded it, and Save refuses when the file has changed since, with a message to Reload.
- **Save refuses news that wouldn't read back the same**: a post without a title, a `## ` line inside a post's text or the top part (it would start a post of its own; the hint suggests `###` for a subheading), and a file over the portal's 256 KB limit. A `# ` line in a post is allowed, as in a hand-written file, though it ends the newest post early in the patcher's window.
- **No Markdown preview in the app.** A hint lists the Markdown basics, and **Open the News page** opens the portal's page while the server runs, which shows exactly what players see.
