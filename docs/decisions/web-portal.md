# Web portal review

Questions asked about the finished portal (PR #2), one at a time.

- **Forgotten passwords: an admin sets a new one.** There's no email on a LAN server, so the admin page for an account has a "Reset password" form. The admin tells the player the new password and the player changes it on their own account page. Email reset links are out.
- **Bans don't kick players already in game.** The portal only writes the database, so a ban stops the next login. The ban form tells admins to use `@kick` in game for someone who's online. No rathena fork change for this.
- **Birthdate is required at sign-up.** With `char_del_option: 2` the game checks the birthdate when a player deletes a character, so an account without one could never delete characters. Players can change it on their account page.
- **The portal shows the birthdate as the game wants it typed (YYMMDD).** The client's delete box only sends six digits, and the char server compares them to the birthdate without its century, so YYYYMMDD and other formats never match. The account page warns when an account has no birthdate (the admin account made in first-run setup has none), since the game then refuses every delete.
- **Admin pages are for Admins (group 99) only.** Lower GM groups such as Law Enforcement get no portal access. A moderator role (ban and reset, but no GM-level changes) can be added later if needed.
