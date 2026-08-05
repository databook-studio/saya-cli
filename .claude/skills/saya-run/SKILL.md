---
name: saya-run
description: Launch the saya CLI — the interactive REPL or a one-shot subcommand. Use when asked to run, start, or try saya locally (e.g. "run saya", "start the repl", "run a query", "test a connection").
---

# Launching saya

`saya` is a Rust workspace binary (crate `saya-cli`, bin name `saya`). It reads config
from `.saya/config.toml` + `.saya/connections.toml` in the current directory (project
scope) and `~/.config/saya/` (user scope).

## Build

```bash
cargo build -p saya-cli
```

The binary lands at `target/debug/saya`. For a fast dev launch you can also use
`cargo run -p saya-cli -- <args>` (everything after `--` is passed to saya).

## Interactive TUI

Running with **no subcommand** in a real terminal launches the full-screen TUI —
a scrolling transcript, a status bar, and a bordered input box pinned to the
bottom with a slash-command popup that opens as you type `/`:

```bash
cargo run -p saya-cli
```

Keys: **Enter** submits · **Tab** accepts a popup suggestion · **↑/↓** history
(or popup navigation when it's open) · **Alt+Enter** newline · **Esc** dismiss
popup / cancel an in-flight agent request · **Ctrl+C** quit. Agent replies stream
live into the transcript with a spinner. Piped / non-TTY input instead runs a
headless line executor (see the **saya-smoke** skill).

Slash commands (type `/help` for the list):

- `/connect <profile>` · `/connections` · `/include <profile>` · `/exclude <profile>`
- `/provider [name]` · `/model [name]` — bare form lists choices; with an arg, sets it
- `/sql <query>` — run raw SQL against the active profile (read-only enforced)
- `/schema [refresh]` · `/privacy [on|off]` · `/approvals [ask|read-only|never]`
- `/sessions` · `/resume <id>` · `/history` · `/clear`
- `/help [command]` · `/exit`

REPL affordances: persistent input history (↑/↓, Ctrl-R), Tab completion menu for
commands + arguments, and syntax highlighting. These require a real terminal.

Non-slash input is sent to the AI agent (this contacts the configured provider).

## One-shot subcommands

```bash
cargo run -p saya-cli -- ask "which tables are largest?"     # AI query
cargo run -p saya-cli -- query --sql "SELECT 1"              # raw SQL (needs a profile)
cargo run -p saya-cli -- connection list                     # configured profiles
cargo run -p saya-cli -- connection test <profile>           # connectivity check
cargo run -p saya-cli -- connection schema <profile> [--refresh]
cargo run -p saya-cli -- config show --resolved --redacted   # effective config
cargo run -p saya-cli -- config doctor                       # diagnostics
```

Useful global flags: `--profile <name>`, `--include-profile <name>`, `--provider`,
`--model`, `--approval-mode <ask|read-only|never>`, `--format <text|json|ndjson>`,
`--non-interactive`, `--continue`, `--resume <id>`.

## Notes

- Slash commands are offline; only agent prompts and `query`/`connection` touch the
  network or a database.
- If the rich editor can't initialize (bare PTY without cursor-position reporting),
  the REPL automatically falls back to a plain line reader — still fully functional.
- To verify behavior end-to-end without a TTY, use the **saya-smoke** skill.
