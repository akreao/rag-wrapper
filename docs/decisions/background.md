# Running in the background

The tray icon, what closing the window does, and starting the server at login (PR to come).

- **Closing the window keeps the server running.** Steven asked for it: players shouldn't be dropped because the window was closed. The app hides into the system tray while the server or a task runs. With nothing running, closing quits, so the app doesn't linger for nothing.
- **A switch turns it off.** Settings' "Keep the server running when this window is closed" (on by default, `KEEP_RUNNING_ON_CLOSE` in `.env`). Off, closing stops the server and quits as before.
- **The tray menu: Open rag-wrapper, Start server, Stop server, Stop server and quit.** Start and Stop run through the window's own buttons, so their output lands in the Activity pane. Quitting always stops the server: a server with no app to stop it would have no way to be stopped short of Docker commands.
- **A Quit button in the window's header too** (review answer), so quitting takes one click on a desktop that shows no tray icon. It stops the server and exits, like the tray's Stop server and quit.
- **No tray, still reachable.** GNOME without the AppIndicator extension shows no tray icon; opening the app from the applications menu brings the hidden window back (the single-instance plugin), and stopping the server then closing the window quits.
- **Start the server at login** is a switch in the same card, off by default. It writes a freedesktop autostart entry (`~/.config/autostart/`) running the AppImage with `--at-login`; it's refreshed on every launch like the menu entry, so a moved or updated AppImage keeps working. At login the window stays hidden and the app runs Start, waiting up to two minutes for Docker. The window opens when the start can't happen or fails, so a failure isn't silent.
- **Daily backups while running.** Backups used to happen on stop and close "in place of a schedule"; a server left in the tray for days would never get one, so the app also checks every hour and makes the daily backup when none is from the last 24 hours.
