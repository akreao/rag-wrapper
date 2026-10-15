# rAthena overrides

Files here override rAthena's stock configs. The rAthena containers see this folder as `conf/user/`, and on every start `entrypoint.sh` uses each file whose name matches one of rAthena's `conf/import/` files (`battle_conf.txt`, `char_conf.txt`, `map_conf.txt`, `groups.yml` and so on) in place of that import file.

The desktop app's Settings screen writes the settings it shows here: the server name in `char_conf.txt`, the message of the day in `motd.txt` (named in `map_conf.txt`), and player commands in `groups.yml`. It edits only its own lines, so anything else you add to those files stays. Its **Battle config** editor writes `battle_conf.txt`: any option from rAthena's `conf/battle/*.conf` files (exp and drop rates, the death penalty and the rest), one line per option changed from its default. `groups.yml` is the exception: the app rewrites it while it starts with the app's comment, so delete that comment to manage groups by hand.

- `.txt` files use rAthena's `key: value` format, the same as `conf/battle/*.conf` and the other stock files. A key here wins over the stock value.
- `.yml` files replace the import file whole, in rAthena's YAML DB format.
- The stack's own settings (database, addresses, server passwords, MD5 passwords, no `_M`/`_F` sign-up, no PIN codes) always win over this folder.
- The stack starts commands on other players with `*` (`charcommand_symbol: *`), because the 20250716 client keeps `#` for itself. Set `charcommand_symbol` in `battle_conf.txt` to use another symbol.
- The stack starts every new character in the same place, the main Izlude (`start_point: iz_int,18,26`, `start_point_pre: new_1-1,53,111`), rather than one of rAthena's five channel copies. Set `start_point` or `start_point_pre` in `char_conf.txt` to change it.
- `custom_npcs.txt` names the custom NPCs that come with rAthena (`npc/custom/`) to load, one script per line, such as `npc/custom/warper.txt`. `re: ` or `pre-re: ` before a script loads it only in that game mode. The desktop app's Mods screen writes it from its list of rAthena's custom NPCs and keeps any other lines, so you can add scripts it doesn't list.
- Changes apply when the game servers restart. Deleting a file goes back to rAthena's defaults.
- The account portal also reads `server_name` from `char_conf.txt` (mounted read-only), so the name in the `clientinfo.xml` it offers matches the game.
- `portal/news.md` is the account portal's News page, in Markdown; the game servers don't read it. Each `## ` heading starts a post, newest at the top, and the patcher's window shows the newest one. Anything above the first `## ` heading, such as a `# Rules` section, shows only on the News page. Raw HTML in it is left out. Changes show on the next page load, no restart needed. The desktop app's News screen edits it for you.
