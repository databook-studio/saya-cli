# 0.2.0 feature GIF — launch splash + colour-coded status bar. The splash shows
# name, tagline, your configured databases, example prompts and key hints; then
# toggling approval mode and privacy recolours the status bar live (green
# read-only, dimmed privacy). Offline — no gateway/DB call. Run from repo root:
#   vhs docs/features/feat-splash.tape

Output docs/features/feat-splash.gif

Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 45ms

Hide
Type "target/release/saya --env-file .env.saya"
Enter
Sleep 2.5s
Show

# The launch splash, with the colour-coded status bar along the bottom
# (profile in the accent colour, provider/model dimmed, approval + privacy
# coloured by state).
Sleep 3s

# Recolour the status bar live: approval mode ask (amber) -> read-only (green).
Type "/approvals read-only"
Sleep 500ms
Escape
Enter
Sleep 1.8s

# …and privacy on (green) -> off (dimmed).
Type "/privacy off"
Sleep 500ms
Escape
Enter
Sleep 2.5s
