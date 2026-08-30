<p align="center">
  <img src="docs/mascot/saya-shadow.svg" width="170" alt="">
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

saya discovers the schema, writes the SQL, **shows it to you**, and runs it
read-only and bounded against PostgreSQL, MySQL, SQLite, DuckDB, or Snowflake.

## Install

Prebuilt binaries for macOS (Apple Silicon + Intel), Linux, and Windows are on
every [release](https://github.com/databook-studio/saya-cli/releases), with
`SHA256SUMS` to verify them.

```bash
brew install databook-studio/tap/saya   # macOS / Linux
cargo binstall saya-cli                 # prebuilt binary, no compile
cargo install saya-cli                  # from source (builds DuckDB; takes a few minutes)
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

## What you get

- 🖥️ **A real terminal UI** — bottom-pinned input, streaming answers, a `/`
  command popup with fuzzy matching, `@table` schema autocomplete, and copy-out
  with `Ctrl+O` / `Ctrl+Y` / `Ctrl+B`. ([demo](docs/demo.gif))
- 🛡️ **You see the SQL before it runs** — the exact statement appears in the
  approval prompt and the transcript. `--approval-mode` picks `ask`,
  `read-only`, or `never`.
- 🧠 **Memory** — tell saya what a table means once and later questions carry
  it. Facts are typed, bound to the schema shape they depend on, and go stale
  when a column they rest on changes. saya never confirms a fact by itself and
  never picks between contradictions. Off by default.
  → [memory](docs/memory.md)
- 🔌 **Databases** — PostgreSQL, MySQL, SQLite, DuckDB, Snowflake — and one
  question can span several connected databases at once, with results side by
  side. ([demo](docs/demo-cross.gif))
- 🤖 **Providers** — Ollama, OpenAI, OpenAI-compatible gateways, Anthropic,
  Gemini.
- ⚙️ **Scriptable** — piped or non-TTY input runs headless with text, JSON, or
  NDJSON output, and typed exit codes.

## Safety

saya is read-only in two layers. Every statement is parsed and rejected if it
writes, and the database session itself is opened read-only — Postgres
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

`saya --help` and `/help` in the TUI are generated from the code, so they are
always current — start there for flags and commands. The guides cover the rest:

| Guide | Covers |
| --- | --- |
| [Installation](docs/installation.md) | binaries, Homebrew, cargo, building from source |
| [Configuration](docs/configuration.md) | config layers, environment variables, state paths |
| [Connections](docs/connections.md) | every database type, TLS modes, secret references |
| [Providers](docs/providers.md) | every supported provider and its settings |
| [Commands](docs/commands.md) | the CLI surface and output formats |
| [Querying databases](docs/querying-databases.md) | worked examples, cross-database queries |
| [Memory](docs/memory.md) | what saya remembers, and the trust model behind it |

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
