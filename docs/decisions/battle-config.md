# Battle config editor

The desktop app's battle config editor (roadmap feature 12): a window of its own for every option in rAthena's `conf/battle/*.conf` files, which replaces the exp rate fields on the Settings screen.

- **Edits go in `overrides/battle_conf.txt`, never in rAthena's own files.** rAthena reads that file after every battle conf file, so it wins, and a server update or rebuild can't undo a change. Only options changed from their default get a line, and setting one back to its default removes the line.
- **The options and comments come from the server's image.** The editor copies `conf/battle/` out of the rAthena image the first time it opens after a build or pull, so what it lists always matches the rAthena commit that runs. Before the first build there's nothing to show, and it says to start the server once.
- **Exp rates live only in the editor.** The Settings screen's base and job exp fields are gone; drop rates, the death penalty and the rest stay on Settings. Settings now writes a rate's keys only when that rate changes there, so a value set for one key alone in the editor (say boss card drops) survives a later Settings save.
- **Values are checked the way rAthena reads them**: yes/no (or on/off, true/false) or a whole number, hex allowed, for every option, and one character, not `/`, `%` or `$`, for the two command symbols, which must differ. rAthena's own minimum and maximum for each option aren't in the image, so an out-of-range number is caught by the map server, which logs a warning and uses the default.
- **`charcommand_symbol` defaults to `*`**, the stack's value (see `server-settings.md`), not rAthena's `#`.
