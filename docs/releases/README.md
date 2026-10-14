# Release notes

One file per app version, named after it (`0.2.0.md`), added in the PR that raises the version in `desktop/src-tauri/tauri.conf.json` and `Cargo.toml`. When that PR merges, the Release workflow publishes the AppImage as `v<version>` with the file's text at the top of the release, followed by the pinned rAthena commit and GitHub's list of merged PRs. A version without a file is released with just those.

## Writing them

Patch notes, not a summary: only what changed since the previous version, written for the person running the server. Every line is something they'd notice or need to act on; features that already existed aren't recapped. Link the PR where there is one. Sections, in this order, each only when it has something:

```markdown
## New
- Settings: a "Max level" field for base and job levels (#24).

## Changed
- The Dashboard refreshes every 2 seconds instead of 5.

## Fixed
- "Open folder" did nothing on KDE (#25).

## Server
- rAthena moved from b6f89ab to 4c1d2e0 (upstream fixes for Rune Knight skills). The app offers it as a server update; your database is backed up first.

## Before you update
- Anything to do by hand first, or "Nothing to do."
```

`0.1.0.md` is the exception: the first release has nothing to compare against, so it summarises the app.
