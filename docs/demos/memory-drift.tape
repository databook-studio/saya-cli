# 3/5 — When the schema moves, it notices.
#   vhs docs/demos/memory-drift.tape
Output docs/features/feat-memory-drift.gif
Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 640
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms

Hide
Type "REPO=$(pwd); bash docs/demos/_fixture.sh /tmp/saya-demo-drift && cd /tmp/saya-demo-drift"
Enter
Type "export HOME=/tmp/saya-demo-drift/home; S=$REPO/target/release/saya' --config config.toml --connections connections.toml --profile shop'"
Enter
Type "$S connection schema shop --refresh >/dev/null 2>&1"
Enter
Type "$S contracts remember shop.main.orders --kind time-column --value created_at >/dev/null"
Enter
Type "clear"
Enter
Show

Type "$S contracts list"
Enter
Sleep 2500ms

Type "# Someone renames the column this fact depends on."
Enter
Sleep 1500ms
Type "sqlite3 shop.db 'ALTER TABLE orders RENAME COLUMN created_at TO placed_at'"
Enter
Sleep 1200ms
Type "$S connection schema shop --refresh | tail -1"
Enter
Sleep 3000ms

Type "# Gone from what the model sees:"
Enter
Type "$S contracts list"
Enter
Sleep 2500ms
Type "# Waiting for you instead:"
Enter
Type "$S contracts queue"
Enter
Sleep 3500ms
