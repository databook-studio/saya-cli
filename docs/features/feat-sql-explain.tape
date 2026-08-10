# 0.2.0 feature GIF — raw /sql with right-aligned numeric columns (and the
# Postgres enum-decode fix on `rating`), then /explain for the query plan.
# DB-only (no gateway). Requires the databook postgres container; run from repo root:
#   SAYA_DOCKER_POSTGRES_PASSWORD=databook vhs docs/features/feat-sql-explain.tape

Output docs/features/feat-sql-explain.gif

Set Shell bash
Set FontSize 15
Set Width 1150
Set Height 720
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "target/release/saya --env-file .env.saya"
Enter
Sleep 2.5s
Show
Sleep 1.5s

# Raw SQL in-session. `rating` is a Postgres enum (mpaa_rating) — decoded cleanly
# now — and the count column is right-aligned on its digits.
Type "/sql select rating, count(*) as films from film group by rating order by films desc"
Sleep 700ms
Escape
Enter
Sleep 3.5s

# /explain shows the full, un-truncated query plan (read-only, all dialects).
Type "/explain select rating, count(*) from film group by rating"
Sleep 700ms
Escape
Enter
Sleep 4s
