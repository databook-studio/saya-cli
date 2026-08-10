# 0.2.0 feature GIF — /export the last query's rows to CSV (RFC-4180) or JSON.
# DB-only (no gateway). Requires the databook postgres container; run from repo root:
#   SAYA_DOCKER_POSTGRES_PASSWORD=databook vhs docs/features/feat-export.tape

Output docs/features/feat-export.gif

Set Shell bash
Set FontSize 15
Set Width 1150
Set Height 700
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "target/release/saya --env-file .env.saya"
Enter
Sleep 2.5s
Show
Sleep 1.2s

# Run a query…
Type "/sql select title, rental_rate, length from film order by rental_rate desc, length desc limit 8"
Sleep 700ms
Escape
Enter
Sleep 3s

# …then write those exact rows to a file. CSV uses RFC-4180 escaping; a .json
# extension writes an array of column-keyed objects instead.
Type "/export /tmp/saya-top-films.csv"
Sleep 700ms
Escape
Enter
Sleep 3s
