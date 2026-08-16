# 5/5 — Your team's knowledge, in version control.
#   vhs docs/demos/memory-team.tape
Output docs/features/feat-memory-team.gif
Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 640
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms

Hide
Type "REPO=$(pwd); bash docs/demos/_fixture.sh /tmp/saya-demo-team && cd /tmp/saya-demo-team"
Enter
Type "export HOME=/tmp/saya-demo-team/home; S=$REPO/target/release/saya' --config config.toml --connections connections.toml --profile shop'"
Enter
Type "$S connection schema shop --refresh >/dev/null 2>&1"
Enter
Type "$S contracts remember shop.main.orders --kind time-column --value created_at >/dev/null"
Enter
Type "$S contracts remember shop.main.orders --kind grain --value 'one row per shipped order' >/dev/null"
Enter
Type "mkdir -p .saya/contracts"
Enter
Type "clear"
Enter
Show

Type "$S contracts export .saya/contracts"
Enter
Sleep 2500ms
Type "cat .saya/contracts/*.toml"
Enter
Sleep 4000ms

Type "# Commit that. A teammate clones the repo:"
Enter
Sleep 1500ms
Type "export HOME=/tmp/saya-demo-team/home2; mkdir -p $HOME"
Enter
Type "$S connection schema shop --refresh >/dev/null 2>&1; $S contracts import . --dry-run"
Enter
Sleep 3500ms
Type "$S contracts import ."
Enter
Sleep 3000ms
