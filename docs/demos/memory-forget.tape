# 4/5 — Forgetting means forgotten, and says so.
#   vhs docs/demos/memory-forget.tape
Output docs/features/feat-memory-forget.gif
Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 640
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms

Hide
Type "REPO=$(pwd); bash docs/demos/_fixture.sh /tmp/saya-demo-forget && cd /tmp/saya-demo-forget"
Enter
Type "export HOME=/tmp/saya-demo-forget/home; S=$REPO/target/release/saya' --config config.toml --connections connections.toml --profile shop'"
Enter
Type "$S connection schema shop --refresh >/dev/null 2>&1"
Enter
Type "clear"
Enter
Show

Type "$S contracts remember shop.main.orders --kind time-column --value created_at"
Enter
Sleep 2500ms
Type "ID=$($S contracts list --format json | grep -o 'c-[0-9a-f]*' | head -1)"
Enter
Sleep 800ms

Type "$S contracts forget $ID"
Enter
Sleep 2000ms
Type "$S contracts list"
Enter
Sleep 2000ms

Type "# Remember the same thing again. It does not quietly come back:"
Enter
Sleep 1500ms
Type "$S contracts remember shop.main.orders --kind time-column --value created_at"
Enter
Sleep 3500ms
