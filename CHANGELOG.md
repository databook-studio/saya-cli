# Changelog

All notable changes to SAYA CLI are recorded here. This project follows
[Semantic Versioning](https://semver.org).

## Unreleased

### Repository note

Two internal skill files — `.claude/skills/saya-run/SKILL.md` and
`.claude/skills/saya-smoke/SKILL.md` — shipped in the `v0.1.0` initial public
release and were removed in `0.3.0`. They are absent from every current tree,
but a deleted file stays retrievable from git history, so they can still be
read from a clone. Both describe how to launch the CLI and smoke-test the REPL
locally; neither contains credentials or infrastructure detail. The exposure is
recorded and accepted rather than repaired, since removing it would mean
rewriting published release history. See
[RELEASING.md](RELEASING.md#internal-only-paths-must-not-reach-a-public-ref).

### Changed — read this before upgrading

Six changes alter behaviour you may be relying on. Three of them can stop
SAYA starting or connecting on a setup that worked in 0.3.0.

- **An unknown key in `config.toml` or `connections.toml` is now an error.**
  Previously a typo fell back to the default silently — worst case a typo'd
  `sslmodee` dropped TLS enforcement with no signal. The error names the
  offending key and lists the valid ones.

  This bites on upgrade if your config carries a key that no longer exists.
  In particular `retention_days` was removed (it was parsed, merged and
  surfaced in diagnostics while nothing read it), so a config that sets it
  will not start. Delete the key.

- **PostgreSQL `sslmode` now defaults to `require`, not `prefer`.** `prefer`
  lets an active attacker answer the SSL request with a refusal and collect
  the credentials in plaintext. A server that does not offer TLS will now be
  refused rather than silently downgraded — including a local development
  Postgres. Set `sslmode = "disable"` explicitly for those; see
  [connections.md](docs/connections.md). Note `require` encrypts but does not
  verify the certificate: use `verify-full` where you need that.

- **The project layer is no longer trusted for security-critical settings.**
  A repository's `.saya/config.toml` can no longer set `ai.base_url`,
  `ai.api_key`, `ai.allow_data_sharing` or `run.read_only` — a cloned
  repository is untrusted input, and those four decide where your API key is
  sent, whether rows leave the machine, and whether read-only enforcement
  stays on. SAYA warns when it ignores one. Pass `--trust-project-config` (or
  set `SAYA_TRUST_PROJECT_CONFIG`) to accept them.

- **Enter no longer approves a tool-approval prompt.** The prompt can appear
  while you are typing your next message, so an implicit Enter must never
  allow SQL to run. Press `y` to allow; `n` or Esc to deny.

- **`saya contracts review` is removed; use `saya contracts decide`.** Two
  commands confirmed or rejected a claim and `review` was the weaker one: its
  `--confirm` and `--reject` were independent flags, so `--confirm --reject`
  (or neither) was caught only at runtime, with a "choose exactly one" error,
  and it took a 64-character claim id. `decide` takes a single `--decision`
  flag clap rejects at parse time, and the short `ki-xxxx` prefix `contracts
  list` prints. `decide` scopes to a profile and takes `--profile`, so a claim outside
  the active one is still reachable. The slash commands
  `/confirm` and `/reject` already route to `decide` and are unchanged.

  Before: `saya contracts review ki-… --confirm`
  After:  `saya contracts decide ki-… --decision confirm`

  Neither command was documented, and nothing routed to `review` but the CLI
  itself, so the removal should not affect recorded workflows; if a script used
  `review`, swap the line above.

- **`saya config show` no longer accepts `--resolved` or `--redacted`.** Both
  flags were accepted and ignored since the initial release: `config show`
  always printed the one view it has — the resolved, redacted configuration —
  regardless of either flag. A script passing `--resolved` or `--redacted`
  succeeds today and will now fail with an "unexpected argument" error; drop
  the flag. The printed output is unchanged, because the flags never had an
  effect. `--redacted` is gone in particular because a flag that implies
  redaction is optional is worse than no flag — redaction is not optional, and
  the flag invited someone to look for the off switch.

### Fixed

- **The read-only guard now applies to the whole statement tree.** A denied
  function reached through `FROM` as a table function, through `LATERAL`, or
  schema-qualified (`pg_catalog.pg_read_file`, `main.read_csv`,
  `x.load_file`) was accepted. `FOR UPDATE` and `FOR SHARE` inside a derived
  table took row locks on a connection reported as read-only. Both are
  closed, on every backend.

- **Transcript redaction no longer fails open.** A credential header was only
  recognised at the start of a line, so a pasted `curl -H 'Authorization:
  Bearer …'` kept its token. A private-key block whose closing marker was cut
  off — the normal case, since transcripts are byte-capped — was written out
  in full.

## 0.3.0 — 2026-08-20 — conversational memory

SAYA learns your data vocabulary from ordinary conversation and carries it
between sessions. Everything else in this release is secondary to that.

### Added

- **Memory and data contracts** — SAYA remembers typed facts about your tables (a
  reporting time column, an alias, a grain, a column's role) and uses them when
  building later queries.

  **It learns from conversation, not commands.** Say "we count a rental by
  `return_date`, not `rental_date`, because a rental only counts once it comes
  back" while asking an ordinary question, and SAYA answers *and* records the
  fact — including the reason. A later session, in a new process, recalls it and
  says so. `saya contracts remember` still exists for stating a fact directly,
  and `contracts list` / `show` / `forget` / `queue` inspect and reverse what is
  known.

  **Nothing is remembered unless you turn it on.** `[memory] mode` defaults to
  `off`, so upgrading changes nothing; `assisted` enables recall and post-turn
  learning. A fact you state yourself is recorded as confirmed; anything SAYA
  merely infers is a candidate, inert until a human confirms it in
  `contracts queue`. Repetition never promotes a candidate.

  **What reached the model is always visible.** Every turn that used memory
  prints a `memory supplied` receipt naming each claim, so recall is inspectable
  rather than asserted. When SAYA departs from a confirmed claim it says so in
  the answer and prints `memory overridden`. When a turn's learning fails or
  times out it prints `memory not recorded` — a fact you stated is never dropped
  in silence. `--verbose` reports the learning boundary itself: the gate
  decision, the objects involved, the outcome, and how many facts were kept.

  Claims are typed and bounded, not free text: they cannot hold SQL, credentials,
  file paths or instructions, and the store refuses those shapes rather than
  scrubbing them. Recalled context reaches the model as quoted, delimited data
  marked untrusted — never as instruction — so a claim can never enable a tool or
  authorise a query. Every statement still passes the same read-only safety layer.

  Claims know the shape of the object they describe, so a schema refresh marks a
  claim stale when a column it depends on is removed, renamed, retyped or becomes
  nullable — and marks nothing at all when the database simply could not be
  reached. Two confirmed claims that contradict each other are both shown and
  marked disputed rather than silently resolved. Forgetting a claim erases its
  value *and* its reason from the database file, not merely from the API's view.

  Also adds scoped preferences (`saya preferences`) for timezone, date grain,
  output style and default profile.

- **SQLite** — connect to SQLite database files with `type = "sqlite"` (`path`,
  optional `read_only` defaulting to true). Read-only by default and through the
  bounded SQL safety layer; `:memory:` is not supported.

### Changed

- `[memory]` is configured by a single `mode` (`off` | `assisted`). The earlier
  `recall` and `learning` keys are gone.
- A single-valued claim (a grain, a default time column, a column's role) can now
  be corrected: re-stating it with a different value replaces the old one and
  names what it displaced, instead of reporting a duplicate and silently keeping
  the first value.
- `contracts remember` confirms in words rather than echoing a 64-character id.
  Machine-readable output still carries the id.

### Removed

- `contracts import` / `contracts export`. Sharing contract files as TOML is a
  separate concern from conversational memory and was cut from this release.

### Fixed

- **Charts now plot decimal columns.** `NUMERIC`/`DECIMAL` values (e.g. `SUM`/`AVG`
  and money columns) decode to JSON strings; `/chart` and `render_chart` treated
  them as non-numeric and silently dropped them, producing empty bar/line/area/
  scatter charts. Numeric strings are now recognized and plotted.

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
