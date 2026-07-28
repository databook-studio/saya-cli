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
saya --format ndjson --non-interactive --approval-mode read-only ask "top customers"
saya config doctor
saya config show --resolved --redacted
saya connection list --connections examples/connections.toml
saya query --profile analytics --sql "select 1"
```

Global flags include `--config`, `--connections`, `--env-file`, `--profile`,
`--include-profile`, `--approval-mode ask|read-only|never`, `--format
text|json|ndjson`, `--non-interactive`, `--allow-data-sharing`, `--no-color`,
and `--verbose`.

Automation never prompts. PostgreSQL `connection test`, `connection schema`,
and `query` are live; query execution allows one parsed read-only statement and
returns code `4` for SQL safety/query failures. `ask` calls Ollama or an
OpenAI-compatible chat endpoint, can inspect schema, and can run bounded SQL
according to the approval policy. Connection and schema failures return code
`3`; provider/agent failures return `5`. JSON writes result envelopes to
stdout and diagnostics to stderr; NDJSON uses one stable envelope per line.
`ask` streams provider text deltas. Text output writes deltas immediately, while JSON and NDJSON
write one valid stable JSON event envelope per delta. `/history` lists saved session IDs
in recent-first order, while `/connect` and `/include` only accept configured
profiles and report selection. `/connect`, `/privacy`, `/model`, and
`/provider` are per-session overrides used by the next prompt; supported
providers are `ollama`, `openai`, and `openai_compatible`. `/include` currently
does not change the active execution profile or run multiple databases.
Interactive prompts do not yet carry full multi-turn model context; resumed
sessions use redacted local history and do not reconstruct provider context.
