# Backups review

Questions asked about database backup and restore (PR #9), one at a time.

- **Backups made by hand are kept apart from automatic ones.** The newest 10 made with "Back up now" are kept, and separately the newest 10 made automatically (before a rebuild or a restore). A run of rebuilds never pushes out a backup someone made on purpose.
- **Backups have their own screen.** A Backups button in the header, beside Settings, opens the list with Back up now and Open folder. Starting a backup or a restore switches to the main screen so its progress shows in the Activity log.
- **A daily backup on stop and quit, and while the server runs.** Stopping the server or quitting the app backs up the database first when there's no backup of any kind from the last 24 hours; since the app can now run for days in the tray, it also checks every hour while it runs, waiting for an empty server for up to 48 hours ([background.md](background.md)). It counts as automatic, so it shares the 10 automatic slots with rebuild and restore backups. If it fails, the server still stops.
