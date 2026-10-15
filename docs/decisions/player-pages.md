# Player pages in the portal

Public pages for players: who's online, rankings, and the server's news and rules (roadmap feature 18). Built in the PR that adds this file.

- **Open to everyone, no log-in.** `/online`, `/rankings` and `/news` are linked from the portal's top bar next to "How to connect". Like the connect page, anyone who can reach the portal can read them.
- **Who's online is the dashboard's list.** Name, class, base and job level and map of every character with `online = 1` in the characters database of the server's mode, the same query as the desktop app's live view (`live.rs`). Admins' characters are listed too.
- **Rankings: by level or by zeny, for every class or one.** The level ranking orders by base level, then base experience, then job level and job experience. "By job" became a class filter that lists only the classes characters actually have. The top 50 are shown. Characters of admin-group accounts (`ADMIN_GROUP`, 99) and of blocked accounts are left out, so GM test characters and a blocked cheater don't top the list. Zeny is shown only on the zeny ranking.
- **News is one Markdown file, `overrides/portal/news.md`.** The portal already mounts `overrides/` read-only, so there's no schema change and nothing new in the stack. It's read on every request, so an edit shows on the next page load. No file, or an empty one, shows "No news yet."
- **Posts are `## ` sections, newest first.** Anything above the first one (for example a `# Rules` section) is the top of the News page. A file that starts with its own `# ` title replaces the page's "News" heading.
- **The patcher's window shows the newest post** in a small scrolling box above the buttons, plus a News link. Links in it open in the player's browser, like the window's other links. The box hides when an update fails, so a long error has room in the 600×360 window.
- **Markdown is rendered with goldmark (GitHub-flavoured), with raw HTML left out.** goldmark's safe default drops `<script>`, inline HTML and `javascript:` links, so the file can't put scripts on the portal. It's the portal's second dependency, chosen over a hand-written renderer for tables, lists and links that behave the way people expect.
- **Editing is by hand for now.** The roadmap's optional editor in the desktop app is left for later; the file lives in the stack folder's `overrides/portal/`.
