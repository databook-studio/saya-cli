# ADR 0001: Release architecture and trust boundaries

- Status: accepted (amended 2026-08-06 — see Amendment)
- Date: 2026-08-03

## Decision

Release candidates are built by a manually triggered GitHub Actions workflow on
native Linux, macOS, and Windows runners. Each build runs formatting, locked
tests, strict locked Clippy, a release build, `saya --version`, and
`config doctor` before uploading a tar archive on Unix or zip archive on
Windows, each with a SHA-256 sidecar. A checksum job verifies every archive
and publishes one `SHA256SUMS` artifact. Local verification can add
`--offline` when the dependency cache is present.

Publishing is a separate job that requires the explicit boolean
`workflow_dispatch` input `publish: true` and completed build/checksum jobs. It
uses `gh release`; no build job publishes implicitly.

Local packaging follows the same smoke contract through `scripts/package.sh`.
Signing is deliberately outside this repository: it requires an external
credential and release-plan gate, so this workflow never fabricates signatures.

## Consequences

The current installation path is source-based (`cargo build --release --locked`
or `cargo install --locked --path crates/saya-cli`). A future public repository
may support `cargo install --locked --git ...`; crates.io and Homebrew are not
release channels yet.
Providers remain an explicit product boundary: Ollama and OpenAI-compatible
chat-completions are implemented; Anthropic, Gemini, and fully offline agent
operation are not.

## Amendment (2026-08-06)

The release is now **tag-triggered and multi-channel**, superseding the parts of
the decision above that describe a manual-dispatch, GitHub-Releases-only,
source-install architecture. The trust boundaries — verified builds, checksummed
archives, and no fabricated signatures — are unchanged.

- Pushing a `vX.Y.Z` tag runs the full pipeline: build → checksums → GitHub
  Release → crates.io publish → Homebrew tap bump. A `workflow_dispatch` with
  `publish: false` remains available for validation-only builds.
- The build matrix adds macOS x86_64, cross-compiled on the Apple Silicon runner
  (GitHub's Intel runners are scarce), alongside Linux x86_64, macOS arm64, and
  Windows x86_64.
- The workspace crates are published to crates.io (`cargo install saya-cli` /
  `cargo binstall saya-cli`), and a Homebrew tap
  ([databook-studio/homebrew-tap](https://github.com/databook-studio/homebrew-tap))
  provides `brew install databook-studio/tap/saya`. The crates.io and Homebrew
  jobs no-op unless their secrets (`CARGO_REGISTRY_TOKEN`, `HOMEBREW_TAP_TOKEN`)
  are configured.
- Signing remains deliberately out of scope. See [`RELEASING.md`](../RELEASING.md)
  for the current end-to-end process.
