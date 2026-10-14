# CI: GitHub Actions

Questions asked about the CI workflow (PR #5), one at a time.

`.github/workflows/ci.yml` runs on every PR and every push to `main`: the portal's `gofmt`, `go vet` and `go test`; `docker compose config` and `shellcheck` on the entrypoint; and a build of the rAthena image, only when `docker/rathena/`, `.env.example`, `compose.yaml` or the workflow changes.

- **CI must pass before a PR merges to `main`.** The checks are required in the branch settings, so a broken portal or a bad rAthena pin can't land by accident. A skipped rAthena image job counts as passed.
- **The desktop app's CI job runs compile checks only.** Each PR runs `cargo check` and `clippy`; the AppImage is built by the separate Release workflow when a release is tagged, and on pull requests that change the packaging ([appimage.md](appimage.md)). The job arrived with the desktop app PR (#3) and also runs its unit tests.
