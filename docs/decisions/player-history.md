# Player history

A chart of players online over the last day and week on the desktop app's Dashboard (roadmap feature 32).

- **The desktop app does the counting.** Every 5 minutes it counts the characters marked online in the server's game mode, the same flag the Online list reads. The server only runs while the app is open, so the app never misses time the server was up. A MariaDB event in the stack was the other option, but it would need the event scheduler turned on and would count stale online flags after a char server crash.
- **Nothing is recorded while the char server or database is down.** rAthena clears every online flag when the char server starts, so the flags mean nothing while it's off. The chart leaves a gap there instead of drawing zero, so "server off" and "nobody played" look different.
- **The counts live in `player-history.tsv` beside the backups**, one `unix-seconds<TAB>count` line per sample, and only the last 7 days are kept (about 2,000 lines). It isn't in the database backups, and a restore doesn't change it.
- **Counts follow the game mode that runs.** Pre-Renewal and Renewal keep separate characters, and the chart counts whichever is running at the time; it doesn't split them.
- **The week view shows each hour's highest count**, so a busy evening isn't averaged away. The day view shows every 5-minute count. Hovering shows the time and count; the header shows the peak.
