# vhs script for a LIVE agent demo (real gateway + database calls).
# Requires these in the environment when you run it (never commit them):
#   SAYA_API_KEY, SAYA_DOCKER_POSTGRES_PASSWORD, SAYA_DOCKER_MYSQL_PASSWORD
# Run from the repo root:
#   SAYA_API_KEY=... SAYA_DOCKER_POSTGRES_PASSWORD=... SAYA_DOCKER_MYSQL_PASSWORD=... \
#     vhs docs/demo-live.tape
# Uses --approval-mode read-only so tool calls auto-approve (no modal to time).
# Sleeps are generous; tune them to your gateway's latency.

Output docs/demo-live.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 55ms

Hide
Type "target/release/saya --approval-mode read-only"
Enter
Sleep 3s
Show
Sleep 1s

# PostgreSQL (pagila): a real question -> schema discovery -> SQL -> streamed answer.
Type "Which 5 films have been rented the most? Show the title and rental count."
Enter
Sleep 12s

# Switch to the MySQL database and ask another question.
Type "/connect docker_mysql"
Escape
Enter
Sleep 2s
Type "How many orders are there, and who are the top 3 customers by order count?"
Enter
Sleep 12s

Ctrl+U
Sleep 300ms
Ctrl+C
Sleep 300ms
Ctrl+C
Sleep 1s
