# CI and release candidates

Pull requests and pushes run `.github/workflows/ci.yml` across Linux, macOS,
and Windows. The verification job runs formatting, the workspace test suite,
and strict Clippy. Live PostgreSQL/MySQL contracts are separate opt-in service
jobs.

Release candidates use `.github/workflows/release-candidate.yml`. Start it
manually from Actions. Each native runner:

1. runs `cargo fmt --check`, locked workspace tests, and strict locked Clippy;
2. builds `saya-cli` in release mode;
3. smoke-tests `saya --version` and `--non-interactive config doctor`;
4. uploads a native tar archive on Unix or zip archive on Windows plus a
   SHA-256 sidecar.

The local verification checklist additionally runs the workspace tests and
Clippy with `--offline` when the dependency cache is present.

The checksum job downloads every build artifact, verifies each sidecar, and
uploads one `SHA256SUMS` manifest. The `publish` input defaults to `false`.
Only an explicit `publish: true` dispatch can run the `gh release create` job,
which depends on both build completion and checksum verification.

Do not treat a green build as a signed release. Signing requires external
credentials and a release plan; no signing step or fake signature is included.
The workflow does not make crates.io or Homebrew releases.

Local parity is available with `scripts/package.sh`, which writes the archive
and `.sha256` file under `dist/` unless `SAYA_PACKAGE_DIR` is set.
