---
name: saya-smoke
description: Smoke-test the saya interactive REPL — verify the Phase F interactivity features (/help, /provider, /model, /sql, /sessions, /resume, did-you-mean, error recovery) work end to end. Use after changing the REPL, slash commands, or session handling, or when asked to smoke-test / sanity-check saya.
---

# Smoke-testing the saya REPL

Two layers: an **automated, offline, hermetic** script that asserts the command
surface via the **headless** (non-TTY) path, and a short **manual TTY** pass for
the full-screen TUI affordances a pipe can't exercise.

Architecture: an interactive terminal launches the ratatui **TUI**; piped/non-TTY
input (the script below, CI, `--format json`) runs the **headless line executor**
(`run_plain_loop`). The two share the same command dispatch, so the script still
validates parsing, `/sql`, sessions/resume, and error recovery.

## 1. Automated smoke test (run this first)

```bash
scripts/smoke-repl.sh
```

It builds `saya` if needed, drives it with a piped slash-command script under a
throwaway `SAYA_SESSION_DIR`/`SAYA_STATE_DB` (so it never touches real user data or
the network), prints the REPL output, and asserts:

- `/help` lists commands incl. `/sql <query>`; `/help connect` shows a usage example
- bare `/provider` lists available providers; bare `/model` reports the model
- did-you-mean: `/conect` → "did you mean /connect?"
- **error recovery**: a bad command does NOT kill the session — a later
  `/resume <bad-id>` still runs and reports "Session not found"
- `/resume <bad-id>` → "Session not found"
- clean exit (code 0)

Exit code 0 = all passed; 1 = one or more assertions failed (the failing line is
printed). Pass a binary path as `$1` to test a specific build, e.g.
`scripts/smoke-repl.sh target/release/saya`.

This script is the regression guard for the "unknown slash command must not tear
down the session" fix — keep it green.

### Ad-hoc piped check (no script)

```bash
printf '/help\n/provider\n/conect prod\n/sql SELECT 1\n/exit\n' | cargo run -q -p saya-cli
```

Piped stdin uses the plain-loop dispatch path — good for command logic, but see
below for what it cannot cover.

## 2. Manual TTY pass (full-screen TUI)

The TUI needs a real terminal, so these must be checked interactively. Run
`cargo run -p saya-cli` in a terminal and verify:

- **Layout**: bordered input box pinned at the bottom, transcript scrolling above,
  status bar between.
- **Auto slash popup**: typing `/` opens the `commands` popup immediately; it
  filters as you type (`/pro` → `provider`); ↑/↓ select, Tab accepts, Esc dismisses.
  `/connect `→ offers configured profiles; `/provider `→ `ollama/openai/…`.
- **Input highlighting**: a line starting with `/` shows the command word in cyan;
  `select * from t` greens SQL keywords. The cursor must stay aligned (no drift),
  including with multibyte characters.
- **History**: run a few lines, press ↑ — prior input returns; ↓ walks forward.
- **Streaming**: ask a plain question → the answer streams into the transcript with
  a spinner in the status bar; **Esc** cancels an in-flight request.
- **Editing**: Alt+Enter adds a newline (box grows); Ctrl+A/E/K and Alt+←/→ work.

This is a plain terminal app — drive it in a terminal pane, not a browser/simulator.

## Interpreting failures

- A missing substring in the automated run points at the specific command whose
  output changed — re-read that command's arm in `slash.rs` / `session_commands.rs`.
- A non-zero exit code with output truncated after some line means a command
  propagated an error instead of being handled in `handle_line` (`session_loop.rs`) —
  that is the exact class of bug this smoke test exists to catch.
- Completion/highlight regressions won't show in the script; they need the manual pass.
