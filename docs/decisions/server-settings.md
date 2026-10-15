# Server settings review

Questions asked about the finished Settings screen (PR #7), one at a time. The screen writes rAthena override files into the stack's `overrides/` folder (see `overrides/README.md`), which apply when the game servers restart.

- **Player commands give the full "Super Player" list.** Turning the setting on gives every player all of rAthena's Super Player commands, @go (free teleport to towns) and @autotrade included. The setting is off by default.
- **Saving never restarts the server by itself.** While the server runs, saving shows a "Restart game servers" button, so several changes can go out in one restart, at a time that suits the players online. A restart disconnects everyone in game; the database and portal keep running.
- **The message of the day is plain ASCII only.** Accented and non-Latin text shows up garbled unless it matches the client's language, so the screen accepts only English letters, digits and punctuation.
- **Commands on other players start with `*`, not `#`.** The 20250716 client keeps `#` for itself, so the stack writes `charcommand_symbol: *` into rAthena's `battle_conf.txt` import, before `overrides/battle_conf.txt`, which can still set another symbol. `@` stays as it is.
- **Rates and the exp lost on death moved to the battle config editor** (see `battle-config.md`). The Settings screen keeps the server name, message of the day and player commands.
