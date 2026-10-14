# rAthena overrides

Files here override rAthena's stock configs. The rAthena containers see this folder as `conf/user/`, and on every start `entrypoint.sh` uses each file whose name matches one of rAthena's `conf/import/` files (`battle_conf.txt`, `char_conf.txt`, `map_conf.txt`, `groups.yml` and so on) in place of that import file.

The desktop app's Settings screen writes the settings it shows here: rates and the death penalty in `battle_conf.txt`, the server name in `char_conf.txt`, the message of the day in `motd.txt` (named in `map_conf.txt`), and player commands in `groups.yml`. It edits only its own lines, so anything else you add to those files stays. `groups.yml` is the exception: the app rewrites it while it starts with the app's comment, so delete that comment to manage groups by hand.

- `.txt` files use rAthena's `key: value` format, the same as `conf/battle/*.conf` and the other stock files. A key here wins over the stock value.
- `.yml` files replace the import file whole, in rAthena's YAML DB format.
- The stack's own settings (database, addresses, server passwords, MD5 passwords, no `_M`/`_F` sign-up) always win over this folder.
- Changes apply when the game servers restart. Deleting a file goes back to rAthena's defaults.
