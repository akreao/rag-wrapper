# Live dashboard

Questions asked about the desktop app's live view (roadmap feature 4, PR #11), one at a time. It shows who's online, uptime, and each container's CPU and memory, in the desktop app only.

- **The live view has its own page.** Players in the sidebar (called Dashboard until the redesign, [ui-redesign.md](ui-redesign.md)) opens it: the characters online, then each container's CPU and memory. The main screen stays as it was, with Services and the Activity log. It only refreshes while the page is open, so `docker stats` doesn't run in the background.
- **Uptime counts from the map server's start.** That's how long players have been able to play without a break; Restart game servers disconnects everyone, so it resets uptime too.
- **CPU is a share of the whole computer.** `docker stats` counts percent of one core, which can read 300% on a busy multi-core PC; the app divides by the number of cores so 100% means the whole computer, like Task Manager. The numbers won't match `docker stats` or `top`.
