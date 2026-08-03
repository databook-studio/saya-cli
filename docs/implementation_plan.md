# SAYA CLI — Open-Source Implementation Plan

> Goal: create `databook-studio/saya-cli`, a standalone open-source Rust CLI
> that brings SAYA's database-aware agent loop to terminals, scripts, and CI.
>
> Planning status: private alpha implementation is active. PostgreSQL, MySQL,
> DuckDB, and Snowflake connectors, typed profiles, interactive terminal mode,
> and hermetic JSON/NDJSON command paths are implemented. The repository remains
> private while the MVP is hardened before open-source publication.

## 1. Product Definition

SAYA CLI will let a user configure one or more database profiles, select a
local or cloud AI provider, ask natural-language questions, inspect schema, and
run bounded read-only SQL from a terminal.

The CLI is a clean extraction of SAYA's concepts, not a Tauri wrapper and not a
copy of DataBook Studio's application state. It will have no dependency on
React, Tauri commands, desktop IPC, DataBook licensing, or private app storage.

### MVP outcomes

- Public repository: `github.com/databook-studio/saya-cli`.
- Binary name: `saya`; package/repository name: `saya-cli`.
- PostgreSQL, MySQL, DuckDB, and Snowflake profiles.
- Ollama, OpenAI-compatible, OpenAI, Anthropic, and Gemini providers.
- Config through TOML, process environment, and an explicit `.env` file.
- Secret references through environment variables and files; keyring is a
  reserved shape and currently unavailable in this runtime.
- Schema discovery, read-only SQL execution, row limits, and query cancellation.
- Single- and multi-database SAYA questions.
- A Claude/Codex-style interactive terminal session launched with `saya`.
- Streaming tool activity, slash commands, approvals, history, and resumable
  sessions.
- Human-readable terminal output plus stable JSON/NDJSON output.
- macOS, Linux, and Windows release artifacts.

### Explicitly out of MVP

- Tauri or desktop UI integration.
- DataBook Studio licensing or Creem integration.
- Agent-executed writes or DDL.
- Notebook UI, charting, or workspace file editing.
- BigQuery, Databricks, and Oracle connectors until real connector
  implementations exist and pass the shared connector contract.
- Skills and Goal mode. Preserve extension points, then add them after the core
  `ask` workflow is stable.

## 2. Source and Licensing Boundary

The current DataBook manifest does not declare an open-source license. Before
moving code, perform a provenance review confirming that the organization owns
the relevant SAYA and connector code and may relicense it publicly.

Recommended license: Apache-2.0, because it includes an explicit patent grant.
The final choice requires owner confirmation before repository publication.

Reuse designs and behavior from:

- `src-tauri/src/features/connections/traits.rs`
- `src-tauri/src/features/connections/errors.rs`
- `src-tauri/src/models/database.rs`
- `src-tauri/src/models/query.rs`
- `src-tauri/src/features/connections/{postgres,mysql,duckdb,snowflake}/`
- `src-tauri/src/features/saya_ai/provider/`
- `src-tauri/src/features/saya_ai/agentic_loop/`
- `src-tauri/src/features/saya_ai/tools/`
- `src-tauri/src/features/saya_ai/safety/`
- `src-tauri/src/features/saya_ai/schema/`

Do not copy:

- Tauri commands, `State`, `Channel`, plugins, permissions, or `lib.rs` setup.
- React code or frontend persistence.
- The desktop `ConnectionManager`, which mixes runtime connections, background
  UI crawling, SQLite app state, and keyring lifecycle.
- The current untyped `HashMap<String, Value>` connector factory.
- License, trial, updater, or desktop-release code.

Security correction during extraction: SQL parse failures must deny execution
by default. The desktop fallback currently allows some unparseable statements
after keyword checks; a public autonomous CLI needs fail-closed behavior.

## 3. Repository Architecture

Use a Cargo workspace with focused crates:

```text
saya-cli/
├── Cargo.toml
├── crates/
│   ├── saya-types/        # shared models and typed errors
│   ├── saya-config/       # discovery, precedence, profiles, secrets
│   ├── saya-connectors/   # DatabaseConnector + feature-gated drivers
│   ├── saya-agent/        # providers, tools, loop, privacy, SQL policy
│   ├── saya-store/        # schema cache, history, audit
│   └── saya-cli/          # clap commands, terminal events, output formats
├── examples/
│   ├── config.toml
│   ├── connections.toml
│   └── .env.example
├── docs/
├── tests/
└── .github/workflows/
```

Dependencies flow inward:

```text
saya-cli -> saya-config
         -> saya-agent -> saya-connectors -> saya-types
                      -> saya-store      -> saya-types
```

Core test seams:

- `ConfigSource`
- `SecretResolver`
- `DatabaseConnector`
- `AiProvider`
- `ToolExecutor`
- `SchemaStore`
- `EventSink`
- `TerminalRenderer`

All production source files target 150 lines or fewer. The hard ceiling is 300
lines for this repository; split modules before reaching it.

## 4. Configuration Contract

### Default locations

Use platform config directories, with these logical paths:

```text
<config-dir>/saya/config.toml
<config-dir>/saya/connections.toml
```

Support project-local files:

```text
.saya/config.toml
.saya/connections.toml
```

Explicit paths are always available:

```bash
saya ask "Top customers by revenue" \
  --config ./ops/saya.toml \
  --connections ./ops/connections.toml \
  --env-file ./.env.saya \
  --profile analytics
```

### Value precedence

Highest priority wins:

1. Explicit CLI flags.
2. Existing process environment.
3. Explicit `--env-file` values.
4. Selected connection profile.
5. Project-local config.
6. User config.
7. Built-in defaults.

Never auto-load `.env`. This prevents surprising local and CI behavior.
Loading an env file must be explicit through `--env-file`.

Profile selection precedence:

1. `--profile`.
2. `SAYA_PROFILE`.
3. `default_profile` in resolved config.
4. The sole available profile.
5. Error when multiple profiles exist and none is selected.

### `config.toml`

```toml
default_profile = "analytics"

[ai]
provider = "ollama"
model = "qwen2.5-coder:14b"
base_url = "http://localhost:11434"
allow_data_sharing = false
api_key = { env = "OPENAI_API_KEY" }

[run]
read_only = true
max_rows = 1000
max_iterations = 12
query_timeout_seconds = 60

[output]
format = "text"
color = "auto"
```

### `connections.toml`

```toml
[profiles.analytics]
type = "postgresql"
host = "localhost"
port = 5432
database = "warehouse"
user = "saya_readonly"
sslmode = "require"
password = { env = "SAYA_ANALYTICS_PASSWORD" }

[profiles.local]
type = "duckdb"
path = "./data/warehouse.duckdb"
read_only = true
```

Snowflake example (file paths are literal; `~` and `$VARS` are not expanded):

```toml
[profiles.snowflake_prod]
type = "snowflake"
account = "xy12345"
user = "jane"
auth_type = "keypair"
private_key = { file = "/absolute/path/to/rsa_key.p8" }
passphrase = { env = "SAYA_SNOWFLAKE_PASSPHRASE" }
warehouse = "ANALYTICS"
database = "PROD"
schema = "PUBLIC"
role = "ANALYST"
```

### Secret policy

- Supported references: `env` and `file`. `keyring` is reserved but currently
  unavailable and returns a typed configuration error.
- Never serialize resolved secrets.
- Reject inline database passwords, private keys, and AI API keys by default.
- Redact connection URLs and provider headers in logs and errors.
- Warn when a referenced secret file has unsafe permissions where supported.
- A keyring shape is retained for forward compatibility, but this runtime
  returns `KeyringUnavailable`; env/file references work in headless Linux and CI.

Environment-only operation is supported:

```bash
SAYA_PROFILE=analytics
SAYA_DB_TYPE=postgresql
SAYA_DB_HOST=localhost
SAYA_DB_PORT=5432
SAYA_DB_NAME=warehouse
SAYA_DB_USER=saya_readonly
SAYA_DB_PASSWORD=secret
SAYA_AI_PROVIDER=ollama
SAYA_AI_MODEL=qwen2.5-coder:14b
saya ask "Show weekly active users"
```

## 5. CLI Surface

Running `saya` without a subcommand starts the interactive terminal interface:

```text
saya
saya --continue
saya --resume <session-id>
```

The non-interactive command surface remains available for scripts and CI:

```text
saya config init
saya config doctor
saya config show --resolved --redacted

saya connection list
saya connection test <profile>
saya connection schema <profile> [--refresh]

saya ask <prompt> --profile <name>
saya ask --file question.md --profile <name>
saya ask <prompt> --profile prod --include-profile staging

saya query --profile <name> --sql "SELECT ..."
saya query --profile <name> --file report.sql
```

### Interactive terminal experience

The interface should preserve normal terminal scrollback rather than requiring
a permanently full-screen UI. It provides:

- Streaming assistant text and structured tool-status events.
- A visible active database profile, included profiles, provider/model, privacy
  state, and approval mode.
- Multi-line prompt editing, command history, and terminal-safe Markdown.
- SQL previews, query timing, row counts, and compact result summaries.
- Ctrl+C to cancel the active model/query operation without losing the session.
- Ctrl+D or `/exit` to close an idle session.
- `--continue` for the most recent session and `--resume` for a selected one.
- Redacted session persistence; raw credentials and provider headers are never
  stored, and raw query rows are excluded by default.

MVP slash commands:

```text
/connect <profile>       switch the primary database
/connections             list and test configured profiles
/include <profile>       add a database to the current run allow-list
/exclude <profile>       remove a secondary database
/provider [name]         inspect or switch AI provider
/model [name]            inspect or switch model
/privacy                 inspect or change cloud-sharing policy
/approvals               inspect or change query approval mode
/schema [refresh]        inspect or refresh schema metadata
/clear                   clear visible conversation context
/history                 list resumable sessions
/help                    show commands and shortcuts
/exit                    end the session
```

Approval modes:

```text
ask          confirm live query execution (interactive default)
read-only    auto-approve bounded read-only queries
never        do not allow live query execution; schema reasoning only
```

Non-interactive runs never prompt. An omitted approval policy resolves to the
safe `never` schema-only mode; `--approval-mode` may explicitly select another
policy when the selected connector is available. Interactive runs default to
`ask`.

Common flags:

```text
--config <path>
--connections <path>
--env-file <path>
--profile <name>
--format text|json|ndjson
--non-interactive
--approval-mode ask|read-only|never
--allow-data-sharing
--no-color
--verbose
```

`--non-interactive` must never prompt or start browser SSO. It fails fast when
secrets or approval are missing, writes diagnostics to stderr, writes data to
stdout, and returns stable exit codes:

```text
0   success
2   invalid arguments or configuration
3   connection or authentication failure
4   query or safety rejection
5   AI provider failure
130 interrupted
```

## 6. Safety and Privacy Defaults

- All agent database tools are read-only in MVP.
- Permit only parsed SELECT, WITH, SHOW, DESCRIBE, and EXPLAIN variants known
  to be non-mutating for the selected dialect.
- Reject multi-statement input by default.
- Enforce a configurable row cap, default 1,000.
- Apply a query timeout and propagate Ctrl-C cancellation.
- Limit repeated tool calls and total agent iterations.
- Cloud row-data sharing defaults to disabled.
- With sharing disabled, cloud providers may receive schema metadata but not
  query rows; the `run_query` tool is hidden and blocked in the dispatcher.
- Multi-database runs use an explicit profile allow-list.
- Audit records contain redacted arguments, timing, status, and row counts,
  never credentials or raw provider headers.

Agent-executed writes remain out of scope until a separate threat model,
approval protocol, transaction strategy, and audit design are accepted.

## 7. Database Rollout

1. PostgreSQL: first connector and contract-test reference.
2. MySQL: reuse the SQLx path and connector contract.
3. DuckDB: embedded/local workflow and fast hermetic tests.
4. Snowflake: isolated milestone for password, key-pair, optional browser SSO,
   query polling, gzip chunks, and SSE-C headers.

The first public stable release should include all four. Snowflake may be
marked beta in earlier pre-releases until live opt-in tests pass.

Future connectors must implement the unified `DatabaseConnector` trait and map
all errors into the typed `ConnectionError` taxonomy.

## 8. TDD Delivery Phases

### Phase 0 — Public repository and governance

- Confirm license and code provenance.
- Create public `databook-studio/saya-cli`.
- Add Apache-2.0 license if approved, README, SECURITY, CONTRIBUTING, code of
  conduct, issue templates, changelog, and architecture decision records.
- Add CI for format, lint, unit tests, dependency policy, secret scanning, and
  supported Rust version.
- Define connector/provider feature flags and release versioning.

Exit: empty workspace builds on macOS, Linux, and Windows; governance files and
branch protection are present.

### Phase 1 — Typed config and secret resolution

- Write failing tests for file discovery, precedence, profile selection,
  environment-only mode, and all secret reference types.
- Implement typed TOML models; do not use arbitrary JSON maps.
- Add redacted `config show` and actionable `config doctor`.

Exit: all precedence and redaction tests pass; no resolved secret can be
serialized or printed by `Debug`.

### Phase 2 — Connector contract and PostgreSQL

- Define `DatabaseConnector`, schema/query models, dialect, and
  `ConnectionError`.
- Write fake-connector contract tests before the implementation.
- Implement PostgreSQL, cancellation, metadata, and bounded query execution.
- Add Docker-backed integration tests, separated from fast unit tests.

Exit: `connection test`, `connection schema`, and `query` work for PostgreSQL.

### Phase 3 — Agent runtime and providers

- Write scripted fake-provider tests for token streaming, tool calls,
  malformed arguments, cancellation, repetition, and iteration limits.
- Extract provider adapters behind `AiProvider`.
- Implement the fail-closed SQL policy and privacy-aware tool catalog.
- Implement schema search, DDL/stats tools, and bounded `run_query`.
- Add `ask` with text, JSON, and NDJSON events.

Exit: SAYA can answer against PostgreSQL without Tauri dependencies, and all
cloud/no-sharing tests prove that rows cannot leave the process.

### Phase 4 — Interactive terminal and sessions

- Define terminal events independently from provider events so UI rendering
  does not leak into `saya-agent`.
- Write failing pseudo-terminal and renderer tests before implementing input,
  streaming, slash commands, cancellation, and resize behavior.
- Implement the scrollback-preserving interactive loop launched by `saya`.
- Add profile/provider/model/privacy/approval state changes through slash
  commands.
- Add redacted session persistence, `--continue`, and `--resume`.
- Persist conversation text and tool metadata; exclude credentials, provider
  headers, and raw query rows by default.
- Ensure Ctrl+C cancels the active operation and returns to the prompt; Ctrl+D
  exits only while idle.

Exit: a user can hold a multi-turn database conversation, inspect every tool
action, cancel safely, close the terminal, and resume the redacted session.

### Phase 5 — MySQL and DuckDB

- Run the same connector contract suite against both engines.
- Add dialect-specific SQL policy tests.
- Use Docker for MySQL and temporary files for DuckDB.

Exit: the four connectors behave consistently through `ask`, `query`, and
schema commands.

### Phase 6 — Snowflake

- Typed TOML/env profiles support keypair, userpass, and externalbrowser auth.
- HTTP fixtures cover auth, polling, errors, gzip chunks, raw unbracketed row
  parsing, SSE-C headers, and redacted browser callback failures.
- Browser SSO is interactive-only, bounded to 120 seconds, and fails clearly
  under `--non-interactive` or piped input.
- Live Snowflake tests remain opt-in and secret-backed; CI stays hermetic.

Exit: fixture suite passes everywhere and opt-in live validation passes before
removing the beta label.

### Phase 7 — Store, docs, packaging, and release candidate

- Extend the session store with a local SQLite schema cache and audit records
  behind `SchemaStore`.
- Write the five-minute README and complete configuration, connections,
  providers, security, CI, and command documentation.
- Add examples with no real credentials.
- Produce checksummed artifacts for macOS, Linux, and Windows; signing remains
  an external credential and release-plan gate.
- Keep crates.io and Homebrew as future channels; document source
  `cargo install --locked --path` for the current release candidate.

Exit: a new user can install, configure, test a connection, and run `saya ask`
from documentation alone.

## 9. Verification Matrix

Required before the first stable release:

- Unit: config precedence, validation, secret redaction, exit-code mapping.
- Contract: every connector through the same behavior suite.
- Security: fail-closed parsing, multi-statement rejection, row caps, timeout,
  cancellation, privacy mode, and allow-list enforcement.
- Provider: local/OpenAI-compatible, OpenAI, Anthropic, and Gemini request/tool
  mapping through HTTP fixtures.
- Integration: Docker PostgreSQL/MySQL, temporary DuckDB, fixture Snowflake.
- CLI golden: help, errors, text output, JSON schema, and NDJSON events.
- Interactive PTY: multi-line input, slash commands, tool streaming, terminal
  resize, Ctrl+C cancellation, idle Ctrl+D, approval modes, and resume.
- Session security: redaction, restrictive file permissions, no credential or
  raw-row persistence, and corrupt-session recovery.
- Cross-platform: macOS, Linux, and Windows build and smoke tests.
- Release: clean-machine install and `saya --version`/`config doctor` checks.

No phase is complete until tests were written first and the relevant full suite
passes. Production source and test helpers must remain modular and focused.

## 10. Terra/Luna Orchestration

The orchestrator owns architecture, task slicing, integration reviews, security
gates, and release acceptance. Implementation work is delegated in disjoint
write scopes.

Terra assignments:

- Workspace architecture and public interfaces.
- Typed config engine and secret resolver.
- Connector trait, errors, PostgreSQL, MySQL, DuckDB, and Snowflake internals.
- Agent loop, providers, SQL safety, privacy, and store interfaces.
- Session lifecycle, cancellation contract, approval policy, and redaction.
- Security-sensitive and cross-crate integration reviews.

Luna assignments:

- Clap command handlers and terminal event rendering.
- Interactive prompt, slash commands, status display, and session UX.
- JSON/NDJSON schemas and golden tests.
- Config-init templates and examples.
- User documentation, CI matrices, packaging, and release workflows.
- Connector/provider fixture and smoke-test expansion after contracts stabilize.

Orchestration rules:

- Each ticket begins with failing tests and has a disjoint write set.
- Terra defines a public contract before Luna builds CLI behavior on it.
- Agents work in isolated branches/worktrees; the orchestrator reviews diffs.
- Cross-crate API changes require an architecture decision record.
- Each phase ends with format, lint, unit, integration, security, and docs
  verification appropriate to that phase.

## 11. Decisions to Confirm Before Phase 0

Recommended defaults are shown in parentheses:

1. License (`Apache-2.0`).
2. Public binary/package naming (`saya` binary, `saya-cli` repository/package).
3. Canonical config format (`TOML`).
4. MVP databases (PostgreSQL, MySQL, DuckDB, Snowflake).
5. Secret policy (references only; no inline secrets by default).
6. Keyring support (deferred; env/file references are the supported MVP shapes).
7. Privacy (read-only; cloud row sharing disabled by default).
8. Snowflake browser SSO (supported only in interactive terminals).
9. Skills and Goal mode (post-MVP).
10. First release channels (GitHub binaries first, then Homebrew and Cargo).

Resolved: the Claude/Codex-style interactive terminal interface is included in
MVP and launches when the user runs `saya` without a subcommand.
