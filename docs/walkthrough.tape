# vhs script for the walkthrough GIF (the TUI part of docs/walkthrough.md).
#   Record with:  cargo build -q -p saya-cli --release --locked
#                 vhs docs/walkthrough.tape   (run from the repo root)
# It drives the interactive TUI on the synthetic demo database through the
# same journey the CLI half records: the duplicate-join trap query via /sql,
# saving it with /investigation save (the name is positional; with no --sql
# the save fills from the last query), listing /investigations, exporting the
# captured rows with /export --snapshot, writing the Markdown report with
# /report, and replaying a saved investigation in the background with
# /investigation run — the status bar names it and offers Esc to detach while
# it runs, and the transcript ends with the evidence line.
#
# The replay target is staged in the hidden phase below with a FIXED id
# (`trap-query`) through `investigation import`: the ids `investigation
# save` generates are hashed from the save time, so a recording cannot type
# one — the imported twin carries the same SQL, and the TUI's own
# `/investigation save` step shows the derived id in its block. Import
# executes nothing; the replay maps it with an explicit `--connection`,
# exactly like the CLI journey's second environment.
#
# The CLI half of the journey (demo → setup refusal → trap question → save →
# export → import → rerun) is recorded in docs/walkthrough.md by
# scripts/walkthrough.sh. Everything here is synthetic, local, and read-only
# — no AI provider and no network. The sandbox lives in a temp dir so the
# tree stays clean; the exported environment persists across the tape's
# commands, and the snapshot/report destinations are a fixed scratch
# directory under /tmp that the hidden phases create and remove. The TUI
# itself is launched from that scratch directory, so the recording names no
# checkout path and no recorder identity.

Output docs/walkthrough.gif

Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

# Build the demo database and stage the fixed-id replay target, in an
# isolated temp sandbox. SAYA_SESSION_DIR keeps session files in the sandbox
# too.
Hide
Type "export W=$(mktemp -d)"
Enter
Type "export SAYA_CONFIG_HOME=$W/config SAYA_STATE_DB=$W/state.sqlite3 SAYA_DEMO_DIR=$W/demo SAYA_INVESTIGATIONS_DIR=$W/investigations SAYA_SESSION_DIR=$W/sessions"
Enter
Type "target/release/saya demo --non-interactive"
Enter
Sleep 2.5s
Type "rm -rf /tmp/saya-walkthrough-record && mkdir -p /tmp/saya-walkthrough-record"
Enter
Sleep 400ms
Type@1ms `printf '%s' '{"format":"saya.investigation","version":1,"id":"trap-query","revision":1,"name":"Trap query","description":"Duplicate-join trap: order rows multiplied by customer_contacts","sql":"SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC","dialect":"sqlite","connection":"demo","objects":["orders","customers","customer_contacts"],"schema_fingerprint":null,"created_unix_ms":0,"updated_unix_ms":0}' > $W/trap-investigation.json`
Enter
Sleep 400ms
Type "target/release/saya --connections $W/demo/connections.toml --non-interactive investigation import $W/trap-investigation.json"
Enter
Sleep 2s

# Launch the TUI read-only on the demo profile — from the neutral scratch
# directory (the binary is copied there), with --workspace binding that
# directory explicitly: no trust modal, and the header names the neutral
# canonical path (/private/tmp/… on macOS) instead of the checkout. The
# terminal is cleared first, so no pre-TUI shell text (sandbox paths, demo
# output) is ever on screen around the app: it is empty when the TUI takes
# over and empty again when it exits.
Type "cp target/release/saya /tmp/saya-walkthrough-record/saya && cd /tmp/saya-walkthrough-record"
Enter
Sleep 1s
Type "clear"
Enter
Sleep 300ms
Type "./saya --connections $W/demo/connections.toml --profile demo --workspace /tmp/saya-walkthrough-record"
Enter
Sleep 3s
Show
Sleep 800ms

# 1) /sql runs the duplicate-join trap: count(*) reports the multiplied rows
#    the join through customer_contacts produces, count(DISTINCT o.id) the
#    real orders. The evidence line names the connection and the scope.
Type@10ms "/sql SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC"
Enter
Wait+Screen /direct sql: demo · 4 rows/
Sleep 1s

# 2) /investigation save <name> takes the name positionally and, with no
#    --sql, saves the last query — the trap above — on the connection that
#    ran it. Its block shows the derived, hash-suffixed id.
Type "/investigation save Trap query"
Enter
Wait+Screen /Saved exactly as shown/
Sleep 800ms

# 3) /investigations lists what is saved: the imported replay target and the
#    just-saved one. (Escape closes the command popup so Enter submits —
#    same as demo.tape.)
Type "/investigations"
Escape
Enter
Wait+Screen /trap-query  1/
Sleep 800ms

# 4) /export --snapshot writes the result already inspected — the /sql
#    capture, held for this session, with no query at all.
Type "/export --snapshot /tmp/saya-walkthrough-record/trap-snapshot.csv"
Enter
Wait+Screen /Exported 4 row/
Sleep 800ms

# 5) /report writes the shareable Markdown report of the same capture: the
#    exact SQL and its provenance, rows omitted by default.
Type "/report /tmp/saya-walkthrough-record/trap-report.md"
Enter
Wait+Screen /Wrote report to/
Sleep 800ms

# 6) /investigation run replays the imported twin in the background — the
#    status bar names it and offers Esc to detach while it runs; on the
#    local demo it finishes almost instantly, and the evidence line lands.
#    The import mapped no connection, so the replay maps one explicitly,
#    exactly as the CLI journey's second environment does.
Type "/investigation run trap-query --connection demo"
Enter
Wait+Screen /saved investigation: demo · 4 rows/
Sleep 1.2s

# Quit and clean up, hidden: the last recorded frame is the TUI itself — the
# post-quit shell repaint (and the cleared screen it restores) is never
# captured. (From a filled line Ctrl+C clears first, twice quits — as in
# demo.tape; from the empty line after the run the first Ctrl+C arms.)
Hide
Ctrl+U
Sleep 400ms
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 800ms
Type "cd /tmp && rm -rf /tmp/saya-walkthrough-record $W"
Enter
Sleep 500ms