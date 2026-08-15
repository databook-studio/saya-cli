# 1/5 — Tell it once. It stops re-deriving.
#   vhs docs/demos/memory-remember.tape   (from the repo root)
Output docs/features/feat-memory-remember.gif
Set Shell bash
Set FontSize 15
Set Width 1100
Set Height 640
Set Padding 20
Set Theme "Catppuccin Mocha"
Set TypingSpeed 70ms

Hide
Type "bash docs/demos/_fixture.sh /tmp/saya-demo-remember && cd /tmp/saya-demo-remember"
Enter
Type "export HOME=/tmp/saya-demo-remember/home; S='<path>/saya --config config.toml --connections connections.toml --profile shop'"
Enter
Type "clear"
Enter
Show

Type "# saya knows the schema. It does not know what the columns MEAN."
Enter
Sleep 1500
Type "$S contracts remember shop.main.orders --kind time-column --value created_at"
Enter
Sleep 2500

Type "$S contracts list"
Enter
Sleep 3000

Type "# One typed fact, bound to this table and this connection."
Enter
Sleep 2000
