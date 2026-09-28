# vhs script for the walkthrough GIF (the TUI part of docs/walkthrough.md).
#   Record with:  vhs docs/walkthrough.tape   (run from the repo root)
# It drives the interactive TUI on the synthetic demo database: `/sql`, then
# a background `/investigation run` (the status bar names the replay and
# offers Esc to detach), then quit.
#
# Scope note, recorded on this build: `/investigation save`,
# `/investigations`, `/report`, and `/export` currently panic the TUI in the
# slash-dispatch chain, so they are described in docs/walkthrough.md but are
# deliberately not recorded here. The saved investigation this replay runs
# is created by the headless CLI below (the same `investigation save`
# operation), which is why the TUI never has to.
#
# The CLI half of the journey (demo → setup refusal → trap question → save →
# export → import → rerun) is recorded in docs/walkthrough.md by
# scripts/walkthrough.sh. Everything here is synthetic, local, and read-only
# — no AI provider and no network. The sandbox lives in a temp dir so the
# tree stays clean; the exported environment persists across the tape's
# commands.

Output docs/walkthrough.gif

Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

# Build the demo database and save one investigation, in an isolated temp
# sandbox; capture the printed id for the replay step.
Hide
Type "export W=$(mktemp -d)"
Enter
Type "export SAYA_CONFIG_HOME=$W/config SAYA_STATE_DB=$W/state.sqlite3 SAYA_DEMO_DIR=$W/demo SAYA_INVESTIGATIONS_DIR=$W/investigations"
Enter
Type "target/release/saya demo --non-interactive"
Enter
Sleep 2.5s
Type "export INV=$(target/release/saya --connections $W/demo/connections.toml --profile demo --non-interactive investigation save --name 'Orders in the demo' --sql 'SELECT count(*) AS orders FROM orders' | sed -n 1p)"
Enter
Sleep 2s

# Launch the TUI read-only on the demo profile.
Type "target/release/saya --connections $W/demo/connections.toml --profile demo"
Enter
Sleep 3s
Show
Sleep 800ms

# 1) /sql runs bounded read-only SQL and shows the result with its evidence
#    line (560 orders on the demo fixture).
Type "/sql SELECT count(*) AS orders FROM orders"
Sleep 1.5s
Enter
Sleep 2.5s

# 2) /investigation run replays the saved query in the background — the
#    status bar names the investigation while it runs.
Type "/investigation run $INV"
Sleep 1.5s
Enter
Sleep 2.5s

# Quit (from a filled line Ctrl+C clears first, twice quits — as in demo.tape).
Ctrl+U
Sleep 400ms
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 1s