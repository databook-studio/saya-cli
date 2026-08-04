# Commands

Running `saya` without a subcommand starts the scrollback-preserving terminal
session. It accepts `/help`, `/connect`, `/connections`, `/include`,
`/exclude`, `/provider`, `/model`, `/privacy`, `/approvals`, `/schema`,
`/clear`, `/history`, and `/exit`.

Examples:

```bash
saya config init
saya --profile analytics
saya --profile prod --include-profile staging ask "compare row counts"
saya --continue
saya --resume 1720000000000
saya --format ndjson --non-interactive --approval-mode read-only ask "top customers"
saya config doctor
saya config show --resolved --redacted
saya connection list --connections examples/connections.toml
saya query --profile analytics --sql "select 1"
```

Global flags include `--config`, `--connections`, `--env-file`, `--profile`,
`--include-profile <profile>` (repeatable flag to connect additional read-only databases), `--approval-mode ask|read-only|never`, `--format
text|json|ndjson`, `--non-interactive`, `--allow-data-sharing`, `--no-color`,
and `--verbose`.

Automation never prompts. PostgreSQL, MySQL, DuckDB, and Snowflake `connection
test`, `connection schema`, and `query` commands are live; Snowflake
`externalbrowser` is rejected in automation because it requires an interactive
TTY. Query execution allows one parsed read-only statement and
returns code `4` for SQL safety/query failures. `ask` calls the configured chat
provider (Ollama, OpenAI, an OpenAI-compatible endpoint, Anthropic, or Gemini),
can inspect schema, and can run bounded SQL according to the approval policy. When additional databases are connected via
`--include-profile` or interactive `/include`, the AI agent is informed of the name and SQL
dialect of every connected database in its context and can navigate between them by passing an
optional `connection` argument to its schema-inspection and query tools (with the primary database as default). If a secondary database fails to connect, it is skipped while the primary run continues. Connection and schema failures return code
`3`; provider/agent failures return `5`. JSON writes result envelopes to
stdout and diagnostics to stderr; NDJSON uses one stable envelope per line.
`ask` streams provider text deltas. Text output writes deltas immediately, while JSON and NDJSON
write one valid stable JSON event envelope per delta. `/history` lists saved session IDs
in recent-first order. The slash commands `/connect <profile>`, `/include <profile>`, and `/exclude <profile>` manage live database connections in interactive sessions: `/connect` sets the primary profile, `/include` adds secondary live read-only database connections (skipped if connection fails), and `/exclude` removes them. `/connect`, `/privacy`, `/model`, and
`/provider` are per-session overrides used by the next prompt; supported
providers are `ollama`, `openai`, `openai_compatible`, `anthropic`, and `gemini`.
When attached to a terminal, each interactive prompt is preceded by a one-line
status header showing the active profile, any included databases, the
provider/model, the approval mode, and the privacy (cloud data-sharing) state,
followed by the `saya> ` input marker. Interactive prompts carry bounded prior user/assistant
turns, and `--continue`/`--resume` reconstruct redacted history with saved
provider settings. `/clear` removes the canonical turns as well as visible
context. Tool arguments, responses, credentials, headers, and rows are never
restored into provider history.

`connection schema PROFILE` and interactive `/schema` authenticate and fetch
live metadata before updating the local schema cache. If a later live attempt
fails, the command may return cached metadata only with an explicit stale
diagnostic. `--refresh` and `/schema refresh` invalidate first and therefore
never fall back. Agent schema tools use the same post-live-connection fallback;
cached metadata never enables query execution without a live connector.

`config init` creates `.saya/config.toml` and `.saya/connections.toml` in the
current directory. It is credential-free, refuses to overwrite either file,
and makes a best-effort rollback after an ordinary creation error; it is not
crash-atomic. Use `--format text|json|ndjson` for a stable result envelope;
errors and diagnostics remain on stderr.
