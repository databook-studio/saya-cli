<p align="center">
  <img src="docs/mascot/saya-owl.svg" width="170" alt="">
</p>

<h1 align="center">SAYA CLI</h1>

<p align="center"><em>Ask your database questions in plain language, from the terminal.</em></p>

<p align="center">
  <a href="https://crates.io/crates/saya-cli"><img alt="crates.io" src="https://img.shields.io/crates/v/saya-cli?style=flat-square&color=9d8bf5"></a>
  <a href="https://github.com/databook-studio/saya-cli/actions/workflows/ci.yml"><img alt="CI status" src="https://img.shields.io/github/actions/workflow/status/databook-studio/saya-cli/ci.yml?branch=main&style=flat-square&label=ci"></a>
  <a href="https://crates.io/crates/saya-cli"><img alt="downloads" src="https://img.shields.io/crates/d/saya-cli?style=flat-square"></a>
  <img alt="minimum supported Rust version" src="https://img.shields.io/crates/msrv/saya-cli?style=flat-square">
  <a href="LICENSE"><img alt="license" src="https://img.shields.io/crates/l/saya-cli?style=flat-square"></a>
</p>

<p align="center">
  <a href="#install">Install</a> ·
  <a href="#quick-start">Quick start</a> ·
  <a href="#safety">Safety</a> ·
  <a href="#documentation">Documentation</a>
</p>

<p align="center">
  <img src="docs/demo-live.gif" alt="saya answering a question against a live database" width="100%">
</p>

saya is a database-aware AI agent for the terminal. Ask questions in
plain language or run SQL directly. For agent queries, saya discovers the
schema, shows you the SQL, then runs it read-only with bounded results against
PostgreSQL, MySQL, SQLite, DuckDB, or Snowflake.

## Install

Prebuilt binaries for macOS (Apple Silicon + Intel), Linux, and Windows are on
every [release](https://github.com/databook-studio/saya-cli/releases), with
`SHA256SUMS` to verify them.

```bash
brew install databook-studio/tap/saya   # macOS / Linux
cargo binstall saya-cli                 # prebuilt binary, no compile
cargo install saya-cli                  # from source (builds DuckDB; takes a few minutes)
```

Building from crates.io needs one flag for SQLite's maths functions, which the
released binaries already carry:

```bash
LIBSQLITE3_FLAGS=-DSQLITE_ENABLE_MATH_FUNCTIONS cargo install saya-cli
```

→ [installation](docs/installation.md)

## Quick start

```bash
saya config init                          # starter config in your user config dir (init prints the path)
$EDITOR ~/.config/saya/connections.toml   # point the example profile at your database
export SAYA_ANALYTICS_PASSWORD='...'      # the profile references it; never commit it
saya config doctor                        # secrets resolve? provider reachable?
saya                                      # start the TUI
```

`config init` writes to your user config directory, which saya trusts. Pass
`--project` to write a `.saya/` pair for a repository instead — that layer is
untrusted, so security-critical settings in it are ignored unless you pass
`--trust-project-config`. `config doctor` names what is missing and exits
non-zero when the setup cannot run a query, so a script can tell.

The starter config points at a local [Ollama](https://ollama.com); edit `[ai]`
in `config.toml` for OpenAI, Anthropic, Gemini, or any OpenAI-compatible
gateway.

One-shot, no TUI:

```bash
saya ask "how many orders shipped last week?"
saya query --sql "SELECT count(*) FROM orders"
```

## Work with your data

- **Ask in plain language.** saya inspects your schema, proposes SQL, and
  shows the statement in the transcript. Choose `ask`, `read-only`, or `never`
  with `--approval-mode` to control approval prompts.
- **Run SQL directly.** Use `saya query` for a single read-only statement, or
  use `saya` to open the interactive terminal UI. The UI streams answers,
  offers `/` command search and `@table` schema completion, and supports
  exporting results. ([TUI demo](docs/demo.gif), [export demo](docs/features/feat-export.gif))
- **Connect more than one database.** Work with PostgreSQL, MySQL, SQLite,
  DuckDB, and Snowflake; add profiles to a session so saya can inspect
  and query each connection. ([cross-database demo](docs/demo-cross.gif))
- **Keep useful context.** Optional memory carries typed facts about tables,
  columns, relationships, and metrics into later questions. Facts are bound to
  the schema, and inferred facts wait for your confirmation. Off by default.
  → [Memory](docs/memory.md)
- **Use local or hosted models.** Supported providers include Ollama, OpenAI,
  OpenAI-compatible endpoints, Anthropic, and Gemini.
- **Automate from scripts.** Piped input and non-interactive commands support
  text, JSON, or NDJSON output with documented exit codes.

## Safety

Database queries use two read-only layers. Every SQL
statement is parsed and rejected if it writes, and the database session itself
is opened read-only — Postgres
`default_transaction_read_only`, MySQL `transaction_read_only`, SQLite
`PRAGMA query_only`, a read-only DuckDB open. Results are bounded by a row cap
and byte budgets, and marked when truncated.

Secrets live in your environment or on disk as *references*, never inline in
committed config. Resolved secrets, provider headers, and raw result rows are
structurally excluded from saved sessions.

Neither layer can prove that an arbitrary database function is side-effect
free, and Snowflake has no session read-only switch — so connect with a
least-privilege, read-only database role. → [SECURITY.md](SECURITY.md)

## Documentation

Start with `saya --help` for current flags and commands, or `/help` inside the
TUI. These guides cover setup and common tasks:

| Guide | Covers |
| --- | --- |
| [Installation](docs/installation.md) | binaries, Homebrew, cargo, building from source |
| [Configuration](docs/configuration.md) | config layers, environment variables, state paths |
| [Connections](docs/connections.md) | every database type, TLS modes, secret references |
| [Providers](docs/providers.md) | every supported provider and its settings |
| [Commands](docs/commands.md) | the CLI surface and output formats |
| [Querying databases](docs/querying-databases.md) | worked examples, cross-database queries |
| [Memory](docs/memory.md) | what saya remembers, and the trust model behind it |
| [Security policy](SECURITY.md) | security boundaries and vulnerability reporting |

## Development

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The demo GIFs are generated with [vhs](https://github.com/charmbracelet/vhs)
from the `docs/*.tape` scripts; the live ones need `SAYA_API_KEY` and a
reachable database.

See [CONTRIBUTING.md](CONTRIBUTING.md). Apache-2.0 licensed.
