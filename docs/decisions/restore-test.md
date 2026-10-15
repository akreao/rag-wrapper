# Restore test

Proving a backup would restore, without restoring it: the Test button on the Backups screen (PR #48, roadmap feature 19).

- **A throwaway container, not a second database in the live MariaDB.** The dump carries its own `CREATE DATABASE`/`USE` lines for `ragnarok`, `ragnarok_prere` and `ragnarok_re`, so loading it into the live server would overwrite them. The test runs `rag-wrapper-restore-test` from the stack's own `db` image (so the MariaDB version matches), with `--network none` and no named volume, and removes it and its anonymous volume afterwards.
- **It loads the dump exactly the way Restore does.** Both go through the same code, including the `DROP`/`CREATE DATABASE ragnarok` preamble that lets a backup from before each mode had its own characters database load.
- **What counts as a pass:** every database the dump creates is there after loading, `mariadb-check --check` reports every table OK, and the accounts, characters and inventory can be counted. Accounts leave out the servers' own (`sex = 'S'`). The counts are shown next to the live database's in the Activity log when it's running, not compared automatically: a backup is meant to differ from today's database.
- **The result is kept with the backup.** It shows under the backup's row and is stored in `.restore-tests.json` in the backups folder (a dotfile, so the folder still shows only backups). Results of pruned backups are dropped. A Docker failure or a cancel isn't recorded against the backup, since neither says anything about the file.
- **Closing the app cancels a test**, like a build or start. The container is removed on the way out, and the next test removes any left by a crash before it starts.
