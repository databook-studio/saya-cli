# Changelog

All notable changes to SAYA CLI are recorded here. This project follows
[Semantic Versioning](https://semver.org).

## Unreleased

## 0.2.0 — 2026-08-09

### Added

- **SQL visibility** — the exact SQL a tool is about to run is now shown in the
  approval prompt (both the TUI dialog and the headless `[y/N]` prompt) and
  echoed into the transcript / headless output, so you approve and audit the
  real query text rather than a generic "read-only SQL query" label. In the TUI
  the executed SQL renders as a labelled, multi-line block (broken before each
  major clause) instead of a collapsed one-liner, and the approval panel now
  sits directly above the input box — with the formatted SQL inside it — rather
  than floating in the middle of the screen.
- **Cross-database queries** — a new `bounded_sql_query_all` agent tool runs one
  bounded, read-only query against every connected database in a single
  approval and returns per-database results. Each database runs independently,
  so a dialect mismatch on one is reported alongside the others' successes
  instead of aborting the whole call.
- **Copy & paste from the TUI** — `Ctrl+O` toggles selection mode (releases the
  mouse so your terminal's own drag-select and copy work, with a `SELECT`
  indicator in the status bar); `Ctrl+Y` copies the last answer and `Ctrl+B`
  copies the whole transcript. Copies go to the OS clipboard via the platform
  tool (`pbcopy` / `wl-copy` / `xclip` / `clip`) and also emit OSC 52 so copies
  reach the local clipboard over SSH.
- **Resumed sessions show their history** — resuming a session (via the
  `/sessions` picker or `--resume` / `--continue`) now replays the prior turns
  into the transcript — each question, the tools it ran, and the answer.
- **Launch splash** — before the first question the TUI shows a centered splash
  (name, tagline, your configured databases, example prompts, and key hints)
  instead of an empty panel; it disappears the moment you ask something.
- **Role rail in the transcript** — every transcript line carries a colour-coded
  left rail (you / saya / tool / system / error) so turns read as distinct groups.
- **Colour-coded status bar** — profile in the accent colour, provider/model
  dimmed, approval mode coloured by risk (green read-only, amber ask, red never),
  and privacy coloured.
- **Markdown in answers** — assistant answers render `**bold**`, inline
  `` `code` ``, `#` headings, `-`/`*` bullets, and GitHub-style tables (drawn as
  box tables) instead of raw text.
- **`/export <path>`** — write the last query's results (from `/sql` or an agent
  tool) to a `.csv` or `.json` file. CSV uses RFC-4180 escaping; JSON is an array
  of column-keyed objects.
- **Follow-up refinement** — the agent receives the SQL it most recently ran, so
  a terse follow-up ("now show the lowest instead", "filter to 2023") adapts the
  previous query instead of rediscovering the schema.
- **Charts to interactive files** — `/chart [type]` and a new `render_chart`
  agent tool render the query result as a self-contained, interactive **Chart.js**
  HTML file (bar/line/area/pie/doughnut/scatter) and open it in the browser; the
  AI chooses the chart type when you ask it to visualize data.
- **`/explain [sql]`** — show the `EXPLAIN` query plan for the given SQL, or the
  last query if omitted (full, un-truncated plan text). Read-only; works across
  PostgreSQL, MySQL, DuckDB, and Snowflake.

### Changed

- Numeric columns in result tables are right-aligned so figures line up on their
  digits, while text stays left-aligned.

### Fixed

- **Postgres enum / unknown-type columns** no longer fail a query with an opaque
  "PostgreSQL query failed". User-defined types (e.g. `mpaa_rating`) are decoded
  from their raw text instead of erroring the whole result.

## 0.1.2 — 2026-08-05

### Distribution

- Added an **Intel macOS** (`x86_64-apple-darwin`) build to the release matrix,
  so releases now cover Linux x86_64, macOS arm64, macOS x86_64, and Windows
  x86_64.
- Prepared crates.io publishing: the workspace libraries are now publishable
  (`saya-types`, `saya-config`, `saya-store`, `saya-agent`, `saya-connectors`),
  enabling `cargo install saya-cli`.
- Added `cargo-binstall` metadata so `cargo binstall saya-cli` downloads the
  prebuilt binary instead of compiling.

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
