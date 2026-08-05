# vhs script for a CROSS-DATABASE demo (real gateway + two databases).
# Requires in the environment (never commit them):
#   SAYA_API_KEY, SAYA_DOCKER_POSTGRES_PASSWORD, SAYA_DOCKER_MYSQL_PASSWORD
# Run from the repo root:
#   SAYA_API_KEY=... SAYA_DOCKER_POSTGRES_PASSWORD=... SAYA_DOCKER_MYSQL_PASSWORD=... \
#     vhs docs/demo-cross.tape
# Launches with a primary (postgres) + an included (mysql) database so the agent
# can inspect and compare both in one question.

Output docs/demo-cross.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 820
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "target/release/saya --approval-mode read-only --profile docker_postgres --include-profile docker_mysql"
Enter
Sleep 3s
Show
Sleep 1s

# One question spanning BOTH connected databases.
Type "Compare the two connected databases: how many tables does each have, and list a few table names from each side by side."
Enter
Sleep 16s

Ctrl+U
Sleep 300ms
Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 1s
