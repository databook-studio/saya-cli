# Memory 3/3 — SAYA disagreeing, visibly.
#
# SAYA admits its own limit: a remembered fact is strong context, not an
# instruction the model must blindly obey. When the model determines rental_date
# is more appropriate than return_date, it queries anyway and surfaces the override notice.
#
# LIVE: Vivanti gateway (deepseek-v4-flash) + databook pagila. Recorded 2026-08-17.
# Not byte-reproducible: the model phrases its answer freely.
#   SAYA_DOCKER_POSTGRES_PASSWORD=databook vhs feat-memory-disagreeing.tape

Output "docs/features/feat-memory-disagreeing.gif"

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 820
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 75ms

Hide
# A throwaway HOME so the demo never reads or writes the operator's own memory,
# and a generated config: memory ships OFF, so a demo must turn it on or it
# records SAYA knowing nothing.
Type 'export HOME=$(mktemp -d); sed -e "s/^model = .*/model = \"deepseek-v4-flash\"/" -e "s/^\[run\]/[memory]\nmode = \"assisted\"\n\n[run]/" .saya/config.toml > $HOME/c.toml'
Enter
# Cache the schema before any fact is stored: a claim made against no cached
# schema is fingerprinted against nothing and reads needs_review, not current.
Type 'target/release/saya --config $HOME/c.toml --connections .saya/connections.toml --profile docker_postgres connection schema docker_postgres --refresh >/dev/null 2>&1'
Enter
Type 'target/release/saya --config $HOME/c.toml --connections .saya/connections.toml --profile docker_postgres --env-file .env.saya --approval-mode read-only'
Enter
Sleep 6s
Show
Sleep 2s

Type "/remember pagila.public.rental time-column return_date"
Enter
Sleep 5s

Type "How many rentals were there in each month of 2022?"
Sleep 1200ms
Enter
Sleep 45s
