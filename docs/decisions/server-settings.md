# Server settings review

Questions asked about the finished Settings screen (PR #7), one at a time. The screen writes rAthena override files into the stack's `overrides/` folder (see `overrides/README.md`), which apply when the game servers restart.

- **Player commands give the full "Super Player" list.** Turning the setting on gives every player all of rAthena's Super Player commands, @go (free teleport to towns) and @autotrade included. The setting is off by default.
- **"Exp lost on death" is one value for both base and job exp.** Official is 1%; 0 turns the penalty off. A server that wants different values sets `death_penalty_base` and `death_penalty_job` by hand in `overrides/battle_conf.txt`; the screen shows the base value, and saving from it sets both to that again.
- **Saving never restarts the server by itself.** While the server runs, saving shows a "Restart game servers" button, so several changes can go out in one restart, at a time that suits the players online. A restart disconnects everyone in game; the database and portal keep running.
- **The message of the day is plain ASCII only.** Accented and non-Latin text shows up garbled unless it matches the client's language, so the screen accepts only English letters, digits and punctuation.
