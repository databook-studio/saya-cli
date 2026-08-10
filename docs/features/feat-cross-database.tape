# 0.2.0 feature GIF — cross-database query: one bounded read-only question runs
# against every connected database (bounded_sql_query_all) and returns per-database
# results in a single approval. LIVE: Vivanti gateway (glm-5.2) + both databook DBs.
#   SAYA_DOCKER_POSTGRES_PASSWORD=databook SAYA_DOCKER_MYSQL_PASSWORD=databook \
#     vhs docs/features/feat-cross-database.tape

Output docs/features/feat-cross-database.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 820
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms

Hide
Type "target/release/saya --env-file .env.saya --approval-mode read-only --profile docker_postgres --include-profile docker_mysql"
Enter
Sleep 3s
Show
Sleep 1s

# One question spanning BOTH connected databases (postgres + mysql).
Type "Compare the two connected databases: how many tables does each have? List a few table names from each."
Sleep 700ms
Enter
# Generous real-time wait (two databases + gateway reasoning). The published GIF
# is sped up in post (see build-gifs.sh).
Sleep 55s
