# AGENTS.md — working on saya

The front door for anyone (human or AI agent) changing this codebase. Read this,
then load only the standards module the task touches. Full detail lives in
[`docs/standards/`](docs/standards/README.md).

> `CLAUDE.md` is a symlink to this file, and `.cursor/rules/` points here — one
> source of truth for every agent.

## What saya is

A database-aware AI agent for the terminal: a full-screen TUI over PostgreSQL, MySQL,
SQLite, DuckDB, and Snowflake, with schema discovery and **bounded, read-only** SQL. A small
Rust workspace — **edition 2024, MSRV 1.88**.

```
saya-types → saya-config → {saya-connectors} ┐
           → saya-store    → saya-agent       ├→ saya-cli   (binary: TUI, render, CLI)
```
Dependencies point down. Contracts live in their crate; **all presentation lives in
`saya-cli`**. → [architecture.md](docs/standards/architecture.md)

## The golden path (follow it for every non-trivial change)

1. **Plan first** — scope, crates touched, the tests you'll add, risks — *before*
   editing. `/plan-first` · [workflow.md](docs/standards/workflow.md)
2. **Test-drive** — red → green → refactor; a failing test comes first. `/tdd` ·
   [testing.md](docs/standards/testing.md)
3. **Review** — check the diff against the standards. `/rust-review`

Use `/rust-standards <topic>` to open the right module on demand.

## Non-negotiables

- **Read-only by default.** All SQL goes through the safety layer in
  `saya-connectors/src/safety/`. Never add a bypass. → [security.md](docs/standards/security.md)
- **No secrets in the tree** — no `.env`, keys, session dirs, or raw DB results in
  commits, fixtures, or snapshots. Use secret *references* and redacted fixtures.
- **Behavior changes start with a failing test.** → [testing.md](docs/standards/testing.md)
- **Small files** — new production `.rs` ≤ 150 lines (soft), 250 hard cap.
- **Contracts in their crate; presentation only in `saya-cli`.**
- **Conventional commits**, no `Co-Authored-By` trailer.

## Standards map — load what the task touches

| Task | Module |
| --- | --- |
| Where code lives, adding a crate, splitting a file | [architecture.md](docs/standards/architecture.md) |
| Writing Rust — errors, APIs, 2024-edition features | [rust-style.md](docs/standards/rust-style.md) |
| Adding/changing tests, choosing a test kind | [testing.md](docs/standards/testing.md) |
| SQL execution, secrets, redaction, capabilities | [security.md](docs/standards/security.md) |
| Planning, committing, opening a PR | [workflow.md](docs/standards/workflow.md) |

## Local gate (must pass before pushing)

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked      # or: cargo test --workspace --locked
cargo test --workspace --doc --locked
cargo audit --deny warnings                  # when dependencies changed
```

## Skills for running saya (already in `.claude/skills/`)

`saya-run` (launch the REPL / subcommands) · `saya-smoke` (headless REPL smoke test) ·
`saya-live-test` (end-to-end against real databases).
