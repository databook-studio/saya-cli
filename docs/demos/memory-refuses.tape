# 2/5 — It refuses to guess.
#   vhs docs/demos/memory-refuses.tape
Output docs/features/feat-memory-refuses.gif
Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 640
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms

Hide
Type "REPO=$(pwd); bash docs/demos/_fixture.sh /tmp/saya-demo-refuse && cd /tmp/saya-demo-refuse"
Enter
Type "export HOME=/tmp/saya-demo-refuse/home; S=$REPO/target/release/saya' --config config.toml --connections connections.toml --profile shop'"
Enter
Type "$S connection schema shop --refresh >/dev/null 2>&1"
Enter
Type "clear"
Enter
Show

Type "# A table that does not exist:"
Enter
Sleep 1200ms
Type "$S contracts remember shop.main.custmers --kind alias --value customers"
Enter
Sleep 3000ms

Type "# Two contradictory facts about the same table:"
Enter
Sleep 1200ms
Type "$S contracts remember shop.main.orders --kind grain --value 'one row per shipped order'"
Enter
Sleep 1500ms
Type "$S contracts remember shop.main.orders --kind grain --value 'one row per order line'"
Enter
Sleep 1500ms
Type "$S contracts show shop.main.orders"
Enter
Sleep 3500ms

Type "# Both kept. Neither chosen. That is your call, not its."
Enter
Sleep 2500ms
