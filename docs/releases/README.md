# Release notes

One file per app version, named after it (`0.2.0.md`), added in the PR that raises the version in `desktop/src-tauri/tauri.conf.json` and `Cargo.toml`. When that PR merges, the Release workflow publishes the AppImage as `v<version>` with the file's text at the top of the release, followed by the pinned rAthena commit and GitHub's list of merged PRs. A version without a file is released with just those.
