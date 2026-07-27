# Commands

Running `saya` without a subcommand starts the scrollback-preserving terminal
session. It accepts `/help`, `/connect`, `/connections`, `/include`,
`/exclude`, `/provider`, `/model`, `/privacy`, `/approvals`, `/schema`,
`/clear`, `/history`, and `/exit`.

Examples:

```bash
saya --profile analytics
saya --continue
saya --resume 1720000000000
saya --format ndjson --non-interactive ask "top customers"
saya config doctor
saya config show --resolved --redacted
saya connection list --connections examples/connections.toml
saya query --profile analytics --sql "select 1"
```

Global flags include `--config`, `--connections`, `--env-file`, `--profile`,
`--include-profile`, `--approval-mode ask|read-only|never`, `--format
text|json|ndjson`, `--non-interactive`, `--allow-data-sharing`, `--no-color`,
and `--verbose`.

Automation never prompts. In this alpha, ask/query and live connection/schema
commands return explicit not-implemented events and exit with `5`, `4`, and `3`
respectively. JSON writes result envelopes to stdout and diagnostics to stderr;
NDJSON uses one stable envelope per line. `/history` lists saved session IDs in
recent-first order, while `/connect` and `/include` only accept configured
profiles and report selection rather than claiming a live connection.
