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
saya config show
saya connection list --connections examples/connections.toml
saya query --profile analytics --sql "select 1"
```

Global flags include `--config`, `--connections`, `--env-file`, `--profile`,
`--include-profile <profile>` (repeatable flag to connect additional read-only databases), `--approval-mode ask|read-only|never`, `--format
text|json|ndjson`, `--non-interactive`, `--allow-data-sharing`, `--no-color`,
and `--verbose`.

Automation never prompts. PostgreSQL, MySQL, SQLite, DuckDB, and Snowflake `connection
test`, `connection schema`, and `query` commands are live; Snowflake
`externalbrowser` is rejected in automation because it requires an interactive
TTY. Query execution allows one parsed read-only statement and
returns code `4` for SQL safety/query failures. `ask` calls the configured chat
provider (Ollama, OpenAI, an OpenAI-compatible endpoint, Anthropic, or Gemini),
can inspect schema, and can run bounded SQL according to the approval policy. It
can also render an interactive Chart.js file from a query via the `render_chart`
tool; because that writes a file and opens a browser (`external_side_effect`), it
always requires approval. When additional databases are connected via
`--include-profile` or interactive `/include`, the AI agent is informed of the name and SQL
dialect of every connected database in its context and can navigate between them by passing an
optional `connection` argument to its schema-inspection and query tools (with the primary database as default). If a secondary database fails to connect, it is skipped while the primary run continues. Connection and schema failures return code
`3`; provider/agent failures return `5`. JSON writes result envelopes to
stdout and diagnostics to stderr; NDJSON uses one stable envelope per line.
`ask` streams provider text deltas. Text output writes deltas immediately, while JSON and NDJSON
write one valid stable JSON event envelope per delta. When the provider reports token
usage for a call, `ask` also emits a `usage` event carrying those counts — one per
provider call, labelled `call: "answer"` for the answering rounds and
`call: "extraction"` for the post-turn learning call, so a script can sum the answer's
tokens separately from the learning call's and compute a cache hit rate over the answer
alone. A provider that reports no usage emits no `usage` event at all: absence means
"unknown", not zero, and an unreported cache figure serialises as `null` where a
reported zero serialises as `0`. `/history` lists saved session IDs
in recent-first order. The slash commands `/connect <profile>`, `/include <profile>`, and `/exclude <profile>` manage live database connections in interactive sessions: `/connect` sets the primary profile, `/include` adds secondary live read-only database connections (skipped if connection fails), and `/exclude` removes them. `/connect`, `/privacy`, `/model`, and
`/provider` are per-session overrides used by the next prompt; supported
providers are `ollama`, `openai`, `openai_compatible`, `anthropic`, and `gemini`.
When attached to a terminal, each interactive prompt shows a one-line status
header (active profile, any included databases, provider/model, approval mode,
and privacy/cloud data-sharing state) followed by the `saya> ` input marker,
with command history recall (Up/Down) and standard line editing. Piped input
uses a plain line reader so scripts and CI behave predictably. Interactive prompts carry bounded prior user/assistant
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

`config init` creates `config.toml` and `connections.toml` in your user config
directory; `--project` writes the `.saya/` pair in the current directory
instead. It is credential-free, refuses to overwrite either file,
and makes a best-effort rollback after an ordinary creation error; it is not
crash-atomic. Use `--format text|json|ndjson` for a stable result envelope;
errors and diagnostics remain on stderr.

## Runs: `saya run`

`saya run "<goal>"` starts a long-running, resumable job: the model proposes a
plan (ordered steps), and the engine executes each step against the configured
database, pausing — never silently stopping — when a declared budget trips, a
step keeps failing past its bounded retries, or the process holding the run
dies. A run directory `runs/<id>/` holds the run's spec (`spec.json`), its
bound plan (`plan.json`), the workspace the run's tools may write into, and
the event journal (`events.ndjson`); the state store mirrors statuses for
`saya run list`. The runs root follows `SAYA_RUNS_DIR`, then the platform data
home.

`saya run` is headless by construction: it never prompts. Scopes must be
declared up front with `--allow <scopes>`; a run without `--allow` refuses to
start with exit `2` and creates nothing.

**Today exactly one scope binds: `workspace-write`.** The grammar also parses
`scratch`, `fetch:<scheme>+<host>`, `runner:<program>` and
`endpoint:<role>=<endpoint>`, and each is **refused with a usage error** that
names what is missing, because no tool in a run's universe consumes them yet.
They are refused rather than accepted-and-ignored on purpose: approving a
capability that gates nothing would tell you the model may do something it
cannot. Each becomes available with the slice that wires it.

Budgets come from `[jobs]` in the config, layered with `--budget KEY=VALUE`
(`wall-clock=<seconds>`, `turns=<n>`, `tool-calls=<n>`,
`tokens.<endpoint>=<n>`); a zero ceiling is refused as a typo, and the
environment is never read for budgets — a run is reproducible from its spec
and config.

Enforcement differs by dimension, and the difference is worth knowing.
`wall-clock` and `tokens` are checked as the run streams and **pause** it
(`BudgetExhausted` / `WallClockExceeded`, exit `6`, resumable). `turns` and
`tool-calls` bound each episode through the agent's own limits, so exhausting
them ends the step rather than pausing the run. Because every episode calls
the single `orchestrator` endpoint today, a `tokens.<endpoint>` ceiling binds
that endpoint; when per-step roles bind, attribution follows the call.

**A resume re-arms the full ceiling.** A run that pauses on wall-clock or
tokens and is resumed gets the whole budget again, so `N` resumes can cost
`N ×` the declared ceiling. That is deliberate — a resume is a decision to
spend more — but it is stated here rather than left to be discovered from a
bill. Per-tool-call approval defaults to `read-only`
(read-shaped tools run, side-effecting tools are denied); `--approval-mode`
overrides it. The episodes call the `orchestrator` endpoint from
`[[ai.endpoints]]`.

Management subcommands:

- `saya run list` — every run, most recent first, with status.
- `saya run show <id>` — one run's status, goal, scopes, pause reason, and
  the deliverables its steps recorded, with sizes and digests.
- `saya run log <id>` — the run's journal, one event per line.
- `saya run resume <id>` — continue a paused or crashed run at its first
  incomplete step. A run with a live holder refuses.
- `saya run cancel <id>` — record a run cancelled. A run with a live holder
  refuses; cancel the owning process with Ctrl-C instead.

Exit codes follow the global scheme, plus `6` for a paused run: a run that
stopped incomplete-but-not-failed exits `6` and says how to resume. A run
completing with failures exits by cause — safety/query `4`, provider/agent
`5`, connection/config `3` — never silently `0`.
