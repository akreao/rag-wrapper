# Port conflicts

What the app does when a port the stack needs is already taken, after v0.1.1 failed to start with Docker's "failed to bind host port 0.0.0.0:8080/tcp: address already in use" for the portal.

- **The app checks the ports before starting.** Start, Restart, the address and portal port changes, and a restore's restart first read the published ports from `docker compose config`. A port held by one of this stack's own containers is fine (`up` replaces them). For any other port, a Docker container from another project that publishes it is named with a `docker stop` command; otherwise the app tries to bind it, and if that fails with "address in use" it says another program holds it and gives `sudo ss -ltnp 'sport = :PORT'`. The app can't name a process another user runs without root, so it doesn't try.
- **The app never stops someone else's container or program.** It reports and lets the person decide.
- **Only the portal's port is configurable in the app.** The Connecting card sets `PORTAL_PORT`; the dashboard link and the portal's connect and patcher pages follow it. The game ports (6900, 6121, 5121) and the web server's 8888 stay fixed, because every player's `clientinfo.xml` and patched client point at them.
