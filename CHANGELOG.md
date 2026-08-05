# Changelog

All notable changes to SAYA CLI are recorded here. This project follows
[Semantic Versioning](https://semver.org).

## Unreleased

## 0.1.1 — 2026-08-05

First release published with prebuilt binaries (Linux, macOS, Windows) and
SHA-256 checksums. No user-facing behavior changes versus 0.1.0.

### Dependencies

- Updated `ratatui` 0.29 → 0.30, `toml` 0.8 → 1.1, and `base64` 0.22 → 0.23.

### Build & CI

- Parallelized compilation (removed a one-job throttle) — roughly halved CI and
  release build times for the bundled DuckDB C++ compile.
- Added dependency/build caching (`rust-cache`) and de-duplicated CI runs.
- Release job now builds only (tests and clippy already run on `main`),
  compiling DuckDB once instead of three times.
- Bumped `actions/upload-artifact` and `actions/download-artifact` to current
  major versions.

### Security

- Documented triage of two unfixable/unreachable advisories (`rsa`
  RUSTSEC-2023-0071, `rkyv` RUSTSEC-2026-0235) in `.cargo/audit.toml`.

## 0.1.0 — 2026-08-05

### Interactive full-screen TUI

- Replaced the inline reedline REPL with a full-screen **ratatui TUI**: a
  scrolling transcript, a status bar, and a bordered multi-line input box pinned
  to the bottom.
- Slash-command popup that opens automatically on `/` with **fuzzy** matching;
  Tab/Enter accept, arrow keys navigate, Esc dismisses.
- `@table` / `@table.column` autocomplete from the cached schema of the active
  and included profiles.
- Live streaming answers into the transcript with a spinner, elapsed timer, and
  the currently-running tool; **Esc** cancels an in-flight request.
- Raw SQL in-session via `/sql`, rendered as an aligned table; interactive
  `/sessions` picker (profile / model / turns / age) and resume.
- Tool-approval modal for `approval:ask`; mouse-wheel and PageUp/PageDown
  scrolling; persistent input history; two-stage Ctrl+C; F1 help overlay;
  bracketed paste; input syntax highlighting.
- Non-TTY input (pipes/CI) runs a headless executor; `reedline` and
  `nu-ansi-term` dependencies removed.

### Performance

- Cap rows fed to the model from a query tool at 50 (display path unchanged),
  return a compact schema from `schema_discovery`, and send `temperature` +
  a stable `prompt_cache_key` on OpenAI-compatible requests.
- `[ai].temperature` is now configurable (default `0.1`).

### Earlier

- Added multi-database agent navigation: connect additional read-only databases
  alongside the primary with `--include-profile` (and interactive `/include`),
  and the AI agent inspects and queries any connected database by passing an
  optional `connection` argument to its tools. The agent is told the name and
  SQL dialect of every connected database; a failed secondary connection is
  skipped while the primary run continues.
- Added a native Anthropic (Claude) provider (`provider = "anthropic"`):
  streaming `content_block` parsing, `input_schema` tool declarations, top-level
  `system`, and `x-api-key`/`anthropic-version` headers.
- Added a Google Gemini provider (`provider = "gemini"`): buffered
  `generateContent` with `functionDeclarations`, `systemInstruction`, and the
  `x-goog-api-key` header. All five documented providers (Ollama,
  OpenAI-compatible, OpenAI, Anthropic, Gemini) are now implemented, so
  configuration and runtime agree.
- Added a rich interactive line editor (reedline) with in-session command
  history recall and line editing, plus a status header (active profile,
  included databases, provider/model, approval mode, and privacy state). Piped
  input keeps the plain line reader for predictable scripting/CI.
- Cloud row-data sharing for Anthropic and Gemini is gated on
  `--allow-data-sharing`, consistent with the other cloud providers.
- Added `saya config init` for credential-free project templates.
- Added local archive packaging with checksum and extracted-binary smoke tests.
- Added a manually triggered release-candidate workflow for native CI builds.
- Documented the supported provider, installation, configuration, connection, and
  release boundaries.

## 0.1.0

- Initial private-alpha CLI surface for PostgreSQL, MySQL, DuckDB, Snowflake,
  Ollama, and OpenAI-compatible providers.
