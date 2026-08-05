# Releasing SAYA CLI

SAYA CLI is distributed as **prebuilt binaries via GitHub Releases**. The
internal library crates (`saya-types`, `saya-config`, `saya-connectors`,
`saya-agent`, `saya-store`) are marked `publish = false` and are not published
to crates.io.

## Versioning

- [Semantic Versioning](https://semver.org). While `0.x`, minor bumps may
  contain breaking CLI/config changes; patch bumps are fixes only.
- All crates share one version (bumped together).

## Cadence

- Release when a meaningful set of features or fixes has landed on `main`
  (roughly monthly, or sooner for important fixes).
- The `CHANGELOG.md` `## Unreleased` section is kept current as changes merge.

## Cutting a release

1. Ensure `main` is green (CI: fmt, clippy `-D warnings`, tests on Linux/macOS/
   Windows, MSRV, and the live DB contracts).
2. Move `CHANGELOG.md` `## Unreleased` to `## X.Y.Z — YYYY-MM-DD` and start a new
   empty `## Unreleased`.
3. Bump the version (`version = "X.Y.Z"` in each crate's `Cargo.toml`), then
   `cargo build --workspace --locked` to refresh `Cargo.lock`.
4. Commit (`release: vX.Y.Z`) and tag: `git tag vX.Y.Z && git push --tags`.
5. Run the **Release candidate** workflow (`.github/workflows/release-candidate.yml`,
   `workflow_dispatch`) to build and verify the cross-platform archives. Set its
   `publish` input to `true` to attach them to a GitHub Release.

## Security

- Never commit secrets. `.env*`, `.saya/config.toml`, `.saya/connections.toml`,
  and session files are gitignored; API keys and DB passwords are supplied via
  `SecretRef` references (`{ env = ... }` / `{ file = ... }`).
- Report vulnerabilities per [`SECURITY.md`](SECURITY.md).
