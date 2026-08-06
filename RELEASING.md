# Releasing SAYA CLI

SAYA CLI is distributed through several channels, all fed by one tag-triggered
pipeline:

- **GitHub Releases** — prebuilt archives for Linux x86_64, macOS arm64, macOS
  x86_64, and Windows x86_64, each with a SHA-256 sidecar plus a `SHA256SUMS`
  manifest.
- **crates.io** — `cargo install saya-cli` (from source) and
  `cargo binstall saya-cli` (prebuilt binary).
- **Homebrew** — `brew install databook-studio/tap/saya` (tap:
  [databook-studio/homebrew-tap](https://github.com/databook-studio/homebrew-tap)).

## Versioning

- [Semantic Versioning](https://semver.org). While `0.x`, minor bumps may
  contain breaking CLI/config changes; patch bumps are fixes only.
- All workspace crates share one version and are published together. The release
  workflow reads the version from `crates/saya-cli/Cargo.toml`.

## What a tag triggers

Pushing a `vX.Y.Z` tag runs `.github/workflows/release-candidate.yml`, whose jobs
run in order:

1. **build** — a matrix over Linux / macOS-arm64 / macOS-x86_64 / Windows builds
   `saya` in release mode (macOS x86_64 is cross-compiled on the Apple Silicon
   runner — GitHub's Intel runners are scarce), smoke-tests the native binaries,
   and packages an archive + `.sha256`. Tests and Clippy are **not** re-run here;
   the branch ruleset already gates `main` on the full suite.
2. **checksums** — verifies every sidecar and aggregates one `SHA256SUMS`.
3. **publish** — creates the GitHub Release and attaches the archives.
4. **publish-crates** — publishes the workspace to crates.io in dependency order
   via `scripts/publish-crates.sh` (idempotent: skips already-published versions).
5. **bump-homebrew** — regenerates the tap formula from the published
   `SHA256SUMS` via `scripts/update-homebrew-formula.sh` and pushes it to the tap.

Jobs 4 and 5 no-op unless their secrets are configured, so a release never fails
because a channel is not set up.

## Required secrets

| Secret | Used by | What it is |
|---|---|---|
| `CARGO_REGISTRY_TOKEN` | publish-crates | A crates.io API token with publish scope. |
| `HOMEBREW_TAP_TOKEN` | bump-homebrew | A fine-grained PAT with *Contents: read and write* scoped to `databook-studio/homebrew-tap` only. |

Add both under **Settings → Secrets and variables → Actions**. `GITHUB_TOKEN`
cannot push to the separate tap repo, which is why bump-homebrew needs its own
PAT.

## Cutting a release

1. Ensure `main` is green. The branch ruleset requires it: `fmt`, Clippy
   `-D warnings`, and tests on Linux/macOS/Windows, plus the Rust 1.88 MSRV check.
2. Move `CHANGELOG.md` `## Unreleased` to `## X.Y.Z — YYYY-MM-DD` and start a
   fresh `## Unreleased`.
3. Bump the version to `X.Y.Z` in every crate's `Cargo.toml` **and** in the
   internal path-dependency version requirements (the libraries are published
   crates: `saya-store = { path = "...", version = "X.Y.Z" }`). Run
   `cargo check --workspace` to refresh `Cargo.lock`.
4. Open a PR with the version bump + changelog and merge it once green.
5. Tag the merged commit and push:
   ```bash
   git tag vX.Y.Z origin/main
   git push origin vX.Y.Z
   ```
   The pipeline then builds, publishes the GitHub Release, publishes to crates.io,
   and bumps the Homebrew formula.

### Validate without publishing

Run **Actions → Release → Run workflow** with `publish: false` to build and
smoke-test the archives on all platforms without creating a release or touching
any channel. (crates.io / Homebrew jobs only run on real `v*` tags.)

### Publish scripts (also runnable locally)

- `scripts/publish-crates.sh` — bottom-up crates.io publish; skips
  already-published versions. Needs `cargo login` locally, or
  `CARGO_REGISTRY_TOKEN` in CI. `DRY_RUN=1` to verify only.
- `scripts/update-homebrew-formula.sh X.Y.Z` — regenerate and push the tap
  formula from the release's `SHA256SUMS`. `DRY_RUN=1` to preview the diff.

### Recovering a partial publish

If publish-crates stops mid-way (for example, a crates.io new-crate rate limit),
use **Re-run failed jobs** on the Actions run — the script is idempotent and
resumes from the first unpublished crate. No re-tag is needed.

## Security

- Never commit secrets. `.env*`, `.saya/config.toml`, `.saya/connections.toml`,
  and session files are gitignored; API keys and DB passwords are supplied via
  `SecretRef` references (`{ env = ... }` / `{ file = ... }`).
- crates.io publishes are permanent (versions can be yanked but not deleted or
  overwritten); double-check the version before tagging.
- Report vulnerabilities per [`SECURITY.md`](SECURITY.md).
