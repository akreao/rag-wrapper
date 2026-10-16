# Portal hardening

Fixes to the web portal from the 2026-10-16 audit: slow downloads, rate limits, memory use and database deadlines.

- **Downloads drop only when they stall.** The server's 30 s `WriteTimeout` covered the whole response, so any patch, patcher zip or patcher self-update that took longer was cut off every time. Patch files, the patcher exe and the patcher zips now get 30 s per write instead: a slow download finishes, a stalled one is dropped. HTML pages keep the 30 s limit.
- **Rate limits count attempts, not failures after the fact.** Login (10 per IP per 15 minutes) takes a slot before checking the password and gives it back when the password is right, so parallel requests can't get more guesses than the limit. Sign-ups (5 per IP per hour) take a slot just before the account is made, so a form with a typo doesn't use one up. Refusals are logged with the IP, so a NAT or proxy in front of the portal showing every player as one address is easy to spot.
- **Changing the password is rate-limited too.** 10 wrong current passwords per account per 15 minutes, so someone at a PC left logged in can't guess the password from the account page.
- **No per-username login limit.** It would let anyone lock the admin out of the portal by typing wrong passwords for them. The per-IP limit stays the only one.
- **Visitors get a session only for a form.** Browsing Connect, Online, Rankings or News stores nothing; the login and sign-up forms start a session for their CSRF token. Sessions are capped at 10,000; past that a new one pushes out a visitor's, never a logged-in player's.
- **Database work on a page has a 5 s deadline**, and the pool keeps its 10 connections open, so a stalled database (a backup lock, a long restore) shows the error page instead of piling requests up.
