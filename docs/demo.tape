# vhs script for the README demo GIF.
#   Record with:  vhs docs/demo.tape   (run from the repo root)
# Shows the interactive TUI: the slash popup, fuzzy matching, the help overlay,
# and the input box. It runs only local commands (no live gateway/DB call), so
# the recording is deterministic and repeatable.

Output docs/demo.gif

Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 680
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 90ms

# Launch the TUI (release binary, from the repo root). Hidden so the GIF starts
# directly in the app.
Hide
Type "target/release/saya"
Enter
Sleep 2.5s
Show
Sleep 800ms

# 1) The slash popup opens the instant you type "/".
Type "/"
Sleep 1.5s

# 2) Fuzzy matching: "mdl" still finds /model.
Type "mdl"
Sleep 1.5s
Tab
Sleep 800ms
Enter
Sleep 2s

# 3) Bare /provider lists the available providers.
Type "/provider"
Sleep 900ms
Escape
Enter
Sleep 2s

# 4) "?" opens the keybinding help overlay.
Type "?"
Sleep 3s
Escape
Sleep 700ms

# 5) Ask a question in plain language (the bottom-pinned input box).
Type "which tables track billing?"
Sleep 2s

# Quit (Ctrl+C twice from an empty-ish line, or from a filled line clears first).
Ctrl+U
Sleep 400ms
Ctrl+C
Sleep 400ms
Ctrl+C
Sleep 1s
