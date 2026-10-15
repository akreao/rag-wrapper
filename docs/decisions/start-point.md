# Start point for new characters

Roadmap feature 14: every new character starts in the same place, so nobody lands on one of rAthena's channel copies of Izlude.

- **One start point per mode.** rAthena picks a start point at random from five: `iz_int` and `iz_int01` to `iz_int04` on Renewal, `new_1-1` to `new_5-1` on Pre-Renewal. On Renewal the copy follows the character through the tutorial (`iz_int0N`, then `int_land0N`, whose sailor sends it to `izlude_a` to `izlude_d` and saves it there). A LAN server is too small to need channels, players on different copies can't see each other, and some NPCs and signboards exist only on the main `izlude`. The stack sets `start_point: iz_int,18,26` and `start_point_pre: new_1-1,53,111`, the first of rAthena's own five.
- **A config default, not a rAthena change.** `entrypoint.sh` writes both lines into rAthena's `char_conf.txt` import, before `overrides/char_conf.txt`, which can still set other start points (as with `charcommand_symbol`). No change on the rathena fork's `rag-wrapper` branch, so no pin move.
- **Characters already created stay where they are.** Their save point is on the copy they were sent to. They can move with the Channel Warp Official ("Go to copy 1", Izlude 122,105), or an admin can use the portal's "Send to Prontera". The copies and the Channel Warp NPC stay, for anyone who goes there on purpose.
