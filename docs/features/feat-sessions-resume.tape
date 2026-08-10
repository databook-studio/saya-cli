# 0.2.0 feature GIF — resuming a session replays its prior turns into the
# transcript (each question, the tools it ran, and the answer). The /sessions
# picker lists saved sessions; ↑/↓ select, Enter resumes.
# Needs at least one saved session first (record a live tape before this one).
#   SAYA_DOCKER_POSTGRES_PASSWORD=databook vhs docs/features/feat-sessions-resume.tape

Output docs/features/feat-sessions-resume.gif

Set Shell bash
Set FontSize 14
Set Width 1200
Set Height 800
Set Padding 18
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "target/release/saya --env-file .env.saya --approval-mode read-only"
Enter
Sleep 2.5s
Show
Sleep 1.2s

# Open the session picker.
Type "/sessions"
Sleep 700ms
Escape
Enter
Sleep 2s

# Browse and resume the most recent session…
Down
Sleep 800ms
Up
Sleep 600ms
Enter

# …its history replays into the transcript.
Sleep 4s
