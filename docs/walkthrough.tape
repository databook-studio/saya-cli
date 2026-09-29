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
# it runs, and the transcript ends with the evidence line. A final segment
# shows `saya open` staging a synthetic CSV (the text-typed preview, leading
# zero intact), then relaunches the TUI on the staged profile for one /sql
# over the file.
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
# export → import → rerun → parameters → context → files → MCP) is recorded
# in docs/walkthrough.md by scripts/walkthrough.sh. Everything here is
# synthetic, local, and read-only — no AI provider and no network. The
# sandbox is a FIXED scratch directory under /tmp (with HOME inside it), so
# the recording names no checkout path, no recorder identity, and no
# /Users/… or /var/folders/… path; the exported environment persists across
# the tape's commands, and the snapshot/report destinations live in that
# scratch directory too. The TUI is launched from the scratch directory
# (the binary is copied there), so no checkout path can appear.

Output docs/walkthrough.gif

Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

# Build the demo database and stage the fixed-id replay target, in a FIXED
# scratch sandbox whose HOME is the sandbox too: every path the TUI could
# print resolves inside /tmp/saya-walk. The TUI launches from $W/tui — a
# subdirectory of the sandbox, disjoint from the state roots beside it
# (sessions/runs must sit OUTSIDE the workspace tree, so the workspace is
# not the sandbox root itself). SAYA_SESSION_DIR keeps session files in the
# sandbox too, and SAYA_FILES_DIR is where `saya open` stages the CSV for
# the final segment.
Hide
Type "export W=/tmp/saya-walk"
Enter
Type "rm -rf $W && mkdir -p $W"
Enter
Type "export HOME=$W SAYA_CONFIG_HOME=$W/config SAYA_STATE_DB=$W/state.sqlite3 SAYA_DEMO_DIR=$W/demo SAYA_INVESTIGATIONS_DIR=$W/investigations SAYA_SESSION_DIR=$W/sessions SAYA_RUNS_DIR=$W/runs SAYA_FILES_DIR=$W/files"
Enter
Type "export PS1='$ '"
Enter
Sleep 200ms
Type "target/release/saya demo --non-interactive"
Enter
Sleep 2.5s
Type@1ms `printf '%s' '{"format":"saya.investigation","version":1,"id":"trap-query","revision":1,"name":"Trap query","description":"Duplicate-join trap: order rows multiplied by customer_contacts","sql":"SELECT c.region, count(*) AS joined_rows, count(DISTINCT o.id) AS distinct_orders FROM orders o JOIN customers c ON c.id = o.customer_id JOIN customer_contacts cc ON cc.customer_id = c.id GROUP BY c.region ORDER BY joined_rows DESC","dialect":"sqlite","connection":"demo","objects":["orders","customers","customer_contacts"],"schema_fingerprint":null,"created_unix_ms":0,"updated_unix_ms":0}' > $W/trap-investigation.json`
Enter
Sleep 400ms
Type "mkdir -p $W/tui && cp target/release/saya $W/tui/saya && cd $W/tui"
Enter
Sleep 1s
Type "./saya --connections $W/demo/connections.toml --non-interactive investigation import $W/trap-investigation.json"
Enter
Sleep 2s
Type "printf 'zip,city,amount,note\n02134,Boston,120.50,\n90210,Los Angeles,89.99,ok\n10001,New York,,\n' > $W/tui/deliveries.csv"
Enter
Sleep 400ms

# Launch the TUI read-only on the demo profile — from the neutral scratch
# directory (the binary was copied there in the hidden phase), with
# --workspace binding that directory explicitly: no trust modal, and the
# header names the neutral canonical path (/private/tmp/… on macOS) instead
# of the checkout. The terminal is cleared first, so no pre-TUI shell text
# (sandbox paths, demo output) is ever on screen around the app; the final
# `saya open` segment below shows the shell on purpose, then clears again.
Type "clear"
Enter
Sleep 300ms
Type "./saya --connections $W/demo/connections.toml --profile demo --workspace $W/tui"
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

# 3) /investigations lists what is saved: a filterable popup with one row per
#    saved investigation (age, name, dialect, connection — no ids). The
#    imported replay target shows its staged-epoch age, the just-saved one
#    reads "just now". (Escape closes the command popup so Enter submits —
#    same as demo.tape.)
Type "/investigations"
Escape
Enter
Wait+Screen /saved investigations — type to filter/
Escape
Sleep 800ms

# 4) /export --snapshot writes the result already inspected — the /sql
#    capture, held for this session, with no query at all.
Type "/export --snapshot /tmp/saya-walk/tui/trap-snapshot.csv"
Enter
Wait+Screen /Exported 4 row/
Sleep 800ms

# 5) /report writes the shareable Markdown report of the same capture: the
#    exact SQL and its provenance, rows omitted by default.
Type "/report /tmp/saya-walk/tui/trap-report.md"
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

# 7) The final segment: `saya open` on the synthetic CSV. The TUI quits —
#    visible this time, the segment is about the shell surface — the preview
#    shows the text-typed staging with the leading zero intact, then the TUI
#    relaunches on the generated file_deliveries profile for one /sql over
#    the file. The staged directory name is captured from the files root:
#    the CSV bytes are fixed so the snapshot is deterministic, but the
#    capture keeps the tape honest about the hash it actually got.
Ctrl+U
Sleep 400ms
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 800ms
Type "./saya open deliveries.csv --non-interactive"
Enter
Sleep 2.5s
Hide
Type "export STAGED=$(ls $W/files | head -1)"
Enter
Sleep 400ms
Show
Sleep 400ms
Type "clear"
Enter
Sleep 300ms
Type "./saya --connections $W/files/$STAGED/connections.toml --profile file_deliveries --workspace $W/tui"
Enter
Sleep 3s
Type@10ms "/sql SELECT zip, city FROM deliveries ORDER BY city LIMIT 1"
Enter
Wait+Screen /direct sql: file_deliveries · 1 rows/
Sleep 1.2s

# Quit and clean up, hidden: the last recorded frame is the TUI itself — the
# post-quit shell repaint (and the cleared screen it restores) is never
# captured. (From the empty line the first Ctrl+C arms, the second quits.)
Hide
Ctrl+U
Sleep 400ms
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 800ms
Type "cd /tmp && rm -rf $W"
Enter
Sleep 500ms