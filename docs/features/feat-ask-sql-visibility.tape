# 0.2.0 feature GIF — natural-language question → schema discovery → bounded
# read-only SQL (shown as a labelled, multi-line block: SQL visibility) →
# streamed answer rendered as Markdown (box table). LIVE: Vivanti gateway (glm-5.2)
# + databook pagila. --approval-mode read-only auto-approves so timing is stable.
#   SAYA_DOCKER_POSTGRES_PASSWORD=databook vhs docs/features/feat-ask-sql-visibility.tape

Output docs/features/feat-ask-sql-visibility.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 820
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms

Hide
Type "target/release/saya --env-file .env.saya --approval-mode read-only"
Enter
Sleep 3s
Show
Sleep 1s

# Ask in plain language. The transcript shows the role rail (you / saya / tool),
# the exact SQL as a formatted block, then the streamed Markdown answer.
Type "Which 5 films have been rented the most? Show the title and rental count as a table."
Sleep 700ms
Enter
# Generous real-time wait for the gateway (glm-5.2 reasons for ~20s). The
# published GIF is sped up in post (see build-gifs.sh) so it plays in ~12s.
Sleep 45s
