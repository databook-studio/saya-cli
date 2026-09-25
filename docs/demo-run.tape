# vhs script for the `saya run` demo GIF.
#   Record with:  vhs docs/demo-run.tape   (run from the repo root)
# Shows the headless run surface's guardrails: three refusals and the empty
# list that proves they created nothing. No run is started, so no provider or
# database is ever called — the recording is deterministic and repeatable,
# like docs/demo.tape. It needs a loadable config (`saya config init` once)
# and, for the closing `run list` frame, an empty state store, so it points
# SAYA_STATE_DB at a fresh path (never commit a state file).

Output docs/demo-run.gif

Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 60ms

# A private state store so the closing `run list` is empty on any machine.
Hide
Type 'export SAYA_STATE_DB="$(mktemp -d)/saya-demo-state.sqlite3"'
Enter
Sleep 600ms
Show
Sleep 800ms

# 1) A run without `--allow` refuses before anything exists (exit 2):
#    no run directory, no store row, no prompt.
Type 'target/release/saya run "weekly revenue report"'
Sleep 900ms
Enter
Sleep 2s

# 2) A scope no tool in a run's universe consumes is refused with a usage
#    error that names why (exit 2) — never accepted and silently ignored.
Type 'target/release/saya run "weekly revenue report" --allow scratch'
Sleep 900ms
Enter
Sleep 2.5s

# 3) A zero ceiling is a typo, not an intent — refused, never clamped (exit 2).
Type 'target/release/saya run "weekly revenue report" --allow none --budget turns=0'
Sleep 900ms
Enter
Sleep 2s

# 4) The proof: none of the refusals created anything (exit 0, "no runs yet").
Type 'target/release/saya run list'
Sleep 900ms
Enter
Sleep 1.5s