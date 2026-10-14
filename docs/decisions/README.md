# Decisions

Choices made while building rag-wrapper. Each says what was decided and what it means for the build.

One file per piece of work, named after it. Start with [plan.md](plan.md), the decisions made before the first build.

## Adding decisions

- **New piece of work:** add a new file here, named after the work (`vpn-address-picker.md`, not a number), with a `#` title, a line saying what it covers and which PR, then one bullet per decision. Don't list it in this README: a shared list is the same line every PR would edit.
- **Follow-up on earlier work:** add to that work's file.
- **A decision that replaces an earlier one:** edit the earlier bullet so the file says what's true now, as before.

Keeping each piece of work in its own file means two PRs that both record decisions touch different files and merge without conflicts.
