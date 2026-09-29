# Commands

Running `saya` without a subcommand starts the scrollback-preserving terminal
session. It accepts `/help`, `/connect`, `/connections`, `/include`,
`/exclude`, `/provider`, `/model`, `/privacy`, `/approvals`, `/allow`,
`/grants`, `/schema`,
`/sql`, `/export`, `/report`, `/investigation`, `/investigations`,
`/clear`, `/history`, and `/exit`.

Examples:

```bash
saya config init
saya demo
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
`--include-profile <profile>` (repeatable flag to connect additional read-only databases), `--approval-mode ask|read-only|never|bypass`, `--format
text|json|ndjson`, `--non-interactive`, `--allow-data-sharing`, `--no-color`,
`--allow <scopes>`, `--deny <program>` (repeatable), and
`--verbose`. `--workspace <dir>` (the interactive session only) binds the
session's workspace root explicitly; without it the root is the git worktree
top above the launch directory, and outside any worktree nothing binds.
The host-command lane composes in the interactive session wherever a
workspace root binds (`run_command` runs PATH-resolved programs
unsandboxed — as your user, with your whole filesystem and network; no root,
no lane); `--deny` states the session's deny list
of bare program names, evaluated before every grant, every approval prompt,
and bypass. Under bypass, a hostile workspace file is effectively arbitrary
code execution as the user.

Automation never prompts. PostgreSQL, MySQL, SQLite, DuckDB, Snowflake,
ClickHouse, and BigQuery `connection
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
write one valid stable JSON event envelope per delta. A run of tool calls
collapses on the text surface into one summary line (`▸ N tool calls · ok —
…`; a single call still prints its `Using tool:` / completion lines as
before), so piped-text consumers that grepped `Using tool:` one line per
call must match the `▸` summary or switch to NDJSON, the supported machine
surface — NDJSON is unchanged, one `tool_requested` / `tool_completed`
envelope per call.
When the question's material definition is ambiguous and the confirmed
context does not resolve it, the agent may ask one focused question instead
of assuming: the turn ends with a `clarification_needed` event (the TUI
renders it as a Question block), and `saya ask` exits `6` — the paused
class, "needs input", not an error — so a script can tell the two apart.
Two live limits worth knowing: only the newest collapsed TUI group can be
expanded (the transcript has no per-block cursor, so older groups cannot be
reached), and groups are sparse in practice (assistant text between calls
is a boundary, so most calls stay one-member groups — a real session
collapsed exactly one group). When the provider reports token
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
workspace root, host lane, and privacy/cloud data-sharing state) followed by the
`saya> ` input marker,
with command history recall (Up/Down) and standard line editing. Piped input
uses a plain line reader so scripts and CI behave predictably. Interactive prompts carry bounded prior user/assistant
turns, and `--continue`/`--resume` reconstruct redacted history with saved
provider settings. `/clear` removes the canonical turns as well as visible
context. Tool arguments, responses, credentials, headers, and rows are never
restored into provider history. The status header's `ws:` segment names the
session's bound workspace root — the git worktree top above the launch
directory, or a `--workspace <dir>` statement, pinned into the session record
and re-opened on a resume from anywhere — or `ws:unbound`, the outside-a-worktree
shape where the write-shaped tools are absent and the workspace reads refuse.
A session's workspace is what the file tools and `run_program` children are
contained to; the session's scratch database and lock live outside it, at
`~/.local/share/saya/sessions/<id>/`, where no file tool and no child can
reach them. The status header's `host:` segment names the host-command lane:
`host:unsandboxed` where the lane composed, `host:off` where it did not, plus
`deny:<names>` where the session's deny list is non-empty. The host lane is
the unsandboxed second lane: `run_command` runs PATH-resolved programs
unsandboxed — as your user, with your whole filesystem and network. Its
guarantees are not `run_program`'s: the contained lane's sandbox bounds do
not apply here. Under bypass, a hostile workspace file is effectively
arbitrary code execution as the user. The deny list bounds the direct ask
only — a denied `curl` does not stop an allowed `make` from invoking curl,
nor a renamed copy (`mycurl`, a symlink or copy of curl) asked under its own
spelling: deny matches the exact program name named in the ask, never content
or resolved identity.

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

## Try it: `saya demo`

`saya demo` builds a deterministic synthetic SQLite database — 240
customers, 560 orders, and 123 customer_contacts rows — and opens it
read-only in an interactive session. No AI provider is needed for schema
browsing and `/sql`; asking questions in plain language needs a configured
provider.

The fixture is designed to exercise tricky SQL: nulls in emails and amounts,
a `customer_contacts` table that multiplies rows when joined, order dates
straddling the year boundary (2025-12-31 against 2026-01-01), and statuses
(`active`, `inactive`, `churned`) that disagree with recent-order activity
for some customers — so "active customer" has two defensible meanings:
`customers.status = 'active'`, or ordered in the last 90 days of the data.

```bash
saya demo            # build (or reuse) the fixture, then open the TUI
saya demo --reset    # rebuild the fixture even when one exists on disk
```

The database lives under the platform data directory at `saya/demo/`
(`demo.sqlite3` plus a generated `connections.toml` with a `demo` profile);
set `SAYA_DEMO_DIR` to place it elsewhere. Without a terminal (or with
`--non-interactive`) the command launches nothing: it prints the database
and connections paths, the launch command (`saya --connections … --profile
demo`), and example SQL to try.

## Guided setup: `saya setup`

`saya setup` is an interactive flow: choose an AI provider and a database
profile, review the exact file changes, probe, then confirm. Nothing is
written until you confirm, and no prompt ever asks for a secret value — the
API key is requested only as an environment-variable **name**, and its
presence (never its value) is reported.

- **AI provider** — five choices: ollama (local, default base URL,
  no key), openai, openai_compatible (asks for its base URL), anthropic, and
  gemini; plus skip.
- **Database** — all seven engines: sqlite, duckdb, postgresql, mysql,
  snowflake, clickhouse, and bigquery; plus skip. DuckDB asks about its
  read-only flag. Snowflake offers its three auth paths — keypair (a private
  key file path plus an optional passphrase env name, recommended), browser
  SSO (no secret), or a password taken only as an environment-variable name
  — and the browser-SSO probe opens the browser **only after explicit
  consent** ("This opens your browser to sign in to Snowflake. Continue?");
  declining skips that probe entirely. Warehouse fields not needed by the
  chosen auth (warehouse, database, schema, role) are asked with blank
  skipping each.
- **Review** — the exact TOML that will be written, before anything is
  written. An existing `connections.toml` is appended to: the existing bytes
  are kept as an exact prefix and a new profile block is added (a profile
  name that already exists is refused). An existing `config.toml` is never
  modified — setup prints the `[ai]` snippet for you to apply yourself.
- **Probes** — one at a time, 15 seconds each. The database probe checks the
  connection; the provider probe runs only with explicit consent, states
  that a request will be sent, and sends only the word "ping" — no schema,
  no rows. A failed probe does not block writing; it says so and leaves the
  choice with you.
- **Commit** — files are backed up privately (`.setup-backup/`) before
  replacing, writes go through private temp files and atomic renames, and a
  reload check verifies the result loads. A failed reload restores the
  originals. An interruption mid-commit leaves a recovery marker that later
  startups warn about (stderr); the next `saya setup` offers to restore or
  finish the interrupted write. A restore step that fails says which file it
  could not restore, keeps the recovery marker, and exits `2` — setup never
  reports success it did not achieve.

`saya setup` needs a terminal. With `--non-interactive` or no TTY it prompts
for nothing, writes nothing, and exits `2` with guidance to
`saya config init` (starter templates) or `saya demo` (sample database).
A cancelled flow changes nothing and exits `0`.

The first screen: with no database configured, the TUI leads with
`saya demo`, then `saya setup`; once a schema is cached, the empty state
shows up to three starter questions derived from it.

## Saved investigations: `saya investigation`

A saved investigation is one portable JSON document: an exact, bounded,
read-only SQL statement plus its name, description, dialect, connection
alias, and referenced tables — no credentials, rows, results, prompts, or
machine-specific identity. Saving validates the SQL
with the same read-only gate execution uses but never executes it, never
connects, and never involves an AI provider. **Literals are stored verbatim:
review the SQL before sharing**, and the command says so where you save.

Documents live at one file per id under the platform data directory
`investigations/` (set `SAYA_INVESTIGATIONS_DIR` to place them elsewhere),
mode `0600` in a `0700` directory, capped at 500 documents. Ids are derived
from the name plus a hash suffix, so they are always safe filename stems —
`investigation list` prints them. Per-machine state (the review binding) is
kept separately under `investigations/local/<id>.json` and is never
exported. Every write to the collection — save, import, delete, binding
updates — is exclusive across processes: mutations run under a repository
lock and a create never replaces an existing document, so a conflicting
concurrent writer is refused (`the saved investigation changed underneath
this command; retry`) instead of silently overwriting.

Subcommands:

- `saya investigation save --name <NAME> [--description <TEXT>] [--sql <SQL> | --file <PATH>] [--connection <PROFILE>] [--param-spec <SPEC>]…`
  — store the exact SQL as a new document (revision 1); with neither `--sql`
  nor `--file`, and stdin piped in, the statement is read from stdin. The
  SQL is checked by the read-only safety gate for the target dialect;
  multi-statement or write SQL is refused (exit `4`). Credential-shaped
  content is refused (exit `2`), not redacted — the document keeps SQL
  semantics exactly. The saving profile is recorded locally as the review
  binding.
- `saya investigation list [--limit <N>] [--offset <N>]`
  — one line per saved investigation (id, revision, dialect, connection,
  name), at most 50 per page; unreadable documents appear as warnings, not
  failures.
- `saya investigation show <ID>` — print the exact definition as JSON plus
  this machine's local review binding (the opaque profile identity is never
  printed).
- `saya investigation edit <ID> [--name <TEXT>] [--description <TEXT>] [--sql <SQL> | --file <PATH>] [--param-spec <SPEC>]…`
  — change the stored fields you pass (everything else is kept; the id,
  dialect, and connection alias never change) and store the result as a
  **new revision**. The replacement SQL crosses the same read-only gate and
  credential-shape refusal, and the edit leaves the local review binding on
  the old revision, so the next run needs `--revalidate` — exactly like any
  other edit. At least one field is required.
- `saya investigation delete <ID> [--revision <N>]` — delete the document
  and its local binding; the current revision is checked first, so a
  document changed underneath the command is refused rather than deleted
  unseen.
- `saya investigation export <ID> <PATH> [--overwrite]` — write the portable
  definition to a file for sharing. Only the definition travels; the write
  is atomic and an existing destination needs `--overwrite` (directories and
  symlinks are refused).
- `saya investigation import <PATH>` — read a definition file, validate the
  whole document (including re-gating the SQL), print a preview (SQL,
  dialect, connection requirement), and store it **without a binding**.
  Nothing is executed and no connection is made. Identical id and content is
  an idempotent no-op; the same id with different content is a conflict
  (exit `2`). The first run afterwards needs `--connection`.
- `saya investigation run <ID> [--connection <PROFILE>] [--revalidate] [--report <PATH> [--rows <N>] [--overwrite]] [--param <NAME=VALUE>]…`
  — replay the saved SQL against an explicit local connection: the same
  bounded, read-only query path as `saya query`, with no AI provider
  involved. The target comes only from `--connection` or the stored local
  binding — never the active or default profile. A dialect mismatch is
  refused (exit `2`).

### Parameters for saved investigations

A saved investigation's SQL may bind values by name with `:name`
placeholders — `WHERE region = :region`. Declare them at `save` with
`--param-spec name:type[:required]` (repeatable), where type is `string`,
`integer`, `boolean`, `decimal`, `date`, or `timestamp` and the third part
marks the parameter required (the default is optional):

```bash
saya investigation save --name "orders since" \
  --sql "SELECT count(*) FROM orders WHERE status = :status AND order_date >= :min_date" \
  --param-spec status:string:required \
  --param-spec min_date:date
saya investigation run orders-since-3f9a1c2b \
  --param status=active --param min_date=2026-01-01
```

Only the `:name` placeholder form is accepted (positional `$1`, `?`, and the
rest are refused), and every declaration must appear in the SQL and every
placeholder must be declared. Bind values with `run --param name=value`
(repeatable); the literal `null` binds a typed null, and an omitted optional
parameter binds a typed null too. Values are parsed strictly as their
declared types and bound natively by the engine at execute time — never
spliced into the SQL text — and are **never stored anywhere**: the evidence
record carries the parameter names and a SHA-256 digest of the value set,
never a value. Binding errors — an unknown name, a bad value, a missing
required parameter (the error lists every required name with its type) —
exit `2` before any profile or connection work: no connection is ever made
with a bad binding. Engines that bind
natively: PostgreSQL, MySQL, SQLite, DuckDB, and BigQuery; Snowflake with
keypair auth only (other auth paths refuse parameterised SQL before any
network activity); ClickHouse refuses parameterised SQL — fixed
(parameter-free) SQL works unchanged. `investigation edit --param-spec`
replaces the whole declaration list (a new revision, so the next run needs
`--revalidate`), and `/investigation run <id> --param name=value` works in
the TUI. → [ADR 0005](adr-0005-typed-query-parameters.md)

The review is bound to the definition's revision, the target profile's
identity, and the schema fingerprint of the referenced tables. When any of
the three changes, the run is refused — `review is stale (…)` — until
`--revalidate` is passed; a revalidated run rewrites the binding. This is
deliberate friction: a saved query that silently runs against a changed
table is worse than one that stops. The referenced tables are derived from
the SQL itself at run time — the document's `objects` list is
informational, and `investigation import` refuses a document whose
`objects` do not match its SQL. A table that cannot be resolved against the
live schema, an ambiguous name, or an incomplete dependency analysis makes
the review **unverifiable**: the run is refused (`schema review
unavailable: …`) unless `--revalidate` is passed, and such a run is never
recorded as reviewed — a revalidated unverifiable run rewrites the binding
without a schema fingerprint, and the next unverifiable run is refused the
same way. Quoted names containing dots (`"orders.v1"`) are kept as one
object, never re-split.

`run` writes a Markdown report with `--report <PATH>`: the same report the
TUI's `/report` writes — exact SQL and provenance by default, result rows
only with `--rows <N>` (at most 100). The report is written only after a
successful replay, and an existing destination needs `--overwrite`.

Exit codes follow the global scheme: `0` ok; `2` usage and domain errors
(unknown id, id conflict, stale review, credential-shaped SQL, an unusable
document, a parameter binding error); `3` store unavailable or
connection/config failure; `4` a safety/query refusal. Exit `5` (agent)
never occurs here — replay builds no provider.

## Session commands: `/investigation`, `/export`, `/report`

`/investigation save|list|show|edit|delete|export|import|run` is the same
operation module the `saya investigation` commands run, so the two surfaces
cannot disagree. Bare `/investigations` opens the TUI's searchable picker;
`/investigations --list` prints the text listing instead.
Three TUI-only details:

- `/investigation save <name>` takes the name **positionally** — unlike the
  CLI, which spells it `--name <NAME>`. With neither `--sql` nor `--file` it
  saves the
  **latest successful, concrete query** — a `/sql` result or a direct agent
  SQL query — on the connection that actually ran it. Failed or denied agent
  queries never become selectable, and fan-out (`bounded_sql_query_all`)
  never counts; pass `--sql <SQL>` (or `--file <PATH>`) to save different
  SQL, `--param-spec <SPEC>` to declare parameters, and
  `--connection <PROFILE>` to save against another profile.
- `/investigation run <id>` runs in the background: the status bar shows a
  spinner naming the investigation while the replay runs on a worker, and
  the transcript stays usable. Esc or Ctrl+C detaches it — the late result
  is discarded (it never replaces the capture, never writes a report) and
  the query is **not cancelled: it may still be running on the server**.
  Detaching changes only what the UI waits for; the replay's audit row and
  review binding are written exactly as a foreground run's.
- `/investigation run` takes no `--report` flags in the TUI — write reports
  with `/report`.

### The `/investigations` picker

Bare `/investigations` opens a searchable picker over the saved
investigations instead of printing the text listing (which
`/investigations --list` keeps). The picker loads the collection (up to 500
summaries, in list order, each labelled with its age, name, dialect, and
connection) on a worker, filters as you type over id and display name, and
captures navigation and filter typing until you choose: **Enter** shows the
selected investigation through the same adapter `/investigation show <id>`
uses, **`r`** starts the same background replay `/investigation run <id>`
dispatches (the same worker-permit cap and busy refusal — a replay that
still needs `--connection` fails in the worker with the command's refusal),
and **Esc** closes. While the picker is open a bare `r` is the run key, not
filter text — the trade every single-key action on this surface makes. The
picker never writes to the store.

`/investigation edit <id>` edits as a new revision, exactly like the CLI
command: the fields you pass replace the stored ones and the next run needs
`--revalidate`.

## Files: `saya open`

`saya open` turns a local CSV or Parquet file into a read-only session: the
file is read exactly once (through the workspace containment primitives, no
symlink following, ≤ 32 MiB), hashed, and staged into a private DuckDB
snapshot keyed by its content hash and parse options under the platform data
directory's `saya/files/` (`SAYA_FILES_DIR` overrides; `0600` files in `0700`
directories). A preview prints — the source file name, its SHA-256 prefix,
the size, row and column counts, the delimiter (sniffed when not given),
whether a header row was used, per-column null counts, and an **inferred**
type per column — and the TUI opens over the snapshot exactly as `saya demo`
does: the same bounded, read-only query, evidence, export, and investigation
machinery as any database, with no database server and no config editing.
Without a terminal (or with `--non-interactive`) the command prints the
preview and the launch command and exits.

```bash
saya open file.csv                    # stage (or reuse), preview, open the TUI
saya open file.parquet                # Parquet keeps its native column types
saya open file.csv --delimiter ';' --no-header --reset --typed
saya open --list                      # staged sources, newest first
saya open --cleanup all               # or a SHA-256 prefix naming exactly one
```

- **CSV is stored as text.** Every column lands as VARCHAR — no type is
  guessed at staging time. The preview's type labels (integer, decimal,
  boolean, date, timestamp, or text) come from at most the first 1,000
  sampled rows and are deliberately strict: numerics with leading zeros stay
  text, dates must be exactly `YYYY-MM-DD`, booleans exactly `true`/`false`,
  timestamps RFC 3339 or naive ISO. `--typed` builds a second
  `<table>_typed` table by explicit `TRY_CAST` per inferred non-text column,
  leaving the text table untouched; cast failures are counted and reported
  per column (the value lands NULL), never refused. An empty field stages as
  NULL, and a ragged data row refuses the whole staging.
- **Parquet is metadata-capped before decode.** Row totals (≤ 500,000) and
  columns (≤ 512) are checked against the file's metadata, and nested types
  (`STRUCT`, `MAP`, `UNION`, lists) are **refused** — only flat schemas
  stage. CSV-only flags (`--delimiter`, `--no-header`, `--typed`) are
  refused on Parquet, and a file whose bytes are Parquet but whose name says
  otherwise is refused, never parsed as CSV.
- **Caps are hard refusals**: 32 MiB source, 500,000 rows, 512 columns,
  64 KiB fields, UTF-8 required, and a 30-second staging wall clock. Nothing
  partial is ever left behind.
- **The session sees only the snapshot.** The DuckDB read-only gate refuses
  every file-reading function (`read_csv`, `read_parquet`, `parquet_scan`,
  `parquet_metadata`, `glob`, `sqlite_scan`, and the rest), so the staged
  copy is the only data the session can query.
- **Reuse requires the content and the parse options to match.** A snapshot
  is reused only when the file's SHA-256 **and** the effective parse
  contract — delimiter, header flag, table name, and `--typed` — all match
  this open; changed options (or a renamed copy of the same bytes, whose
  table name differs) stage a separate snapshot of the same content, and the
  preview is always read back from the snapshot's own stored metadata.
  `--reset` restages the matching snapshot. `--list` shows the staged sources
  newest-first; `--cleanup <sha-prefix>|all` removes only validated saya
  snapshots (a prefix naming several snapshots of one content is refused as
  ambiguous; foreign files survive). Cleanup is manual — nothing expires
  automatically.

→ [ADR 0007](adr-0007-file-sources.md)

## MCP: `saya mcp serve`

`saya mcp serve` exposes saya's read-only capabilities to MCP hosts over
stdio: newline-delimited JSON-RPC on stdin/stdout (protocol frames only —
diagnostics go to stderr), exit `0` on stdin EOF. The same read-only safety
gate every other client crosses sits behind every tool, and the surface has
no approval or grant machinery at all: a missing secret refuses, it never
prompts, so MCP profiles need complete, resolvable credentials.

```bash
saya mcp serve --profile analytics --profile staging
saya mcp serve --allow-data-sharing   # rows may leave the machine
```

- **Tools.** `list_profiles` (allowlist names + SQL dialects only), `schema`
  (live discovery with the state-store cache fallback), and `contracts`
  (Active claims only) are always available. `query` (one bounded read-only
  statement → `columns`/`rows`/`truncated` plus evidence) and
  `investigation_run` (replay a saved investigation → result, evidence,
  connection; `params` binds its declared parameters as `{"name": "value"}`
  strings) return rows, so they exist **only when data sharing is allowed** —
  absent from `tools/list` entirely when it is off, and refused at dispatch
  too. The data-sharing gate is the config's `[ai] allow_data_sharing`
  folded with the global `--allow-data-sharing` / `--no-data-sharing` flags.
- **The profile allowlist is decided at startup and never widened.** The
  `serve` subcommand's `--profile` values (repeatable) fix it; with none
  given, the configured default profile is it (and with none configured the
  server starts empty). Every tool that takes a profile is held inside it:
  `schema`, `query`, and `contracts` resolve their profile argument through
  it — a client naming a configured-but-unlisted profile gets `profile not
  available: …` — and `investigation_run` is checked too, before anything
  runs: its effective target (the `profile` argument, else the saved
  binding's profile) must be allowlisted, and a binding saved against an
  unlisted profile refuses with a bare `profile not available`, never
  naming the profile. `list_profiles` reports the allowlist, never a
  configured-but-unlisted profile.
- **Bounds.** Requests ≤ 1 MiB (an oversized line is answered with a
  JSON-RPC error and discarded unread), ≤ 4 tool calls in flight (beyond it,
  a tool-level `busy` error invites a retry), 30 s per call, responses
  ≤ 16 MiB (a `query` payload first narrows by halving rows, refusing only
  when even the rowless payload exceeds the bound).
- **Every failure is a sanitized `isError` result**: credential-shaped
  content is redacted, control characters are stripped, and saya's own
  errors carry no paths — a database engine's own error text may name the
  database file it failed on (for example a missing DuckDB file). Write
  SQL (`DELETE`, `DROP`, …) is refused by the read-only gate,
  and a stale review on `investigation_run` is an error naming
  `--revalidate` — words the client reads, a flag it cannot pass; a replay
  never revalidates from the wire.
- **Cancellation drops the call**: a `notifications/cancelled` stops the
  wait and discards the late response; only `query` on the engines that
  support it stops the actual server-side work.
- **Stdin EOF drops pending work.** The server exits `0` the moment stdin
  closes, without waiting for requests still in flight: a one-shot pipeline
  that writes its JSON-RPC frames and then closes stdin — for example
  `printf '%s\n' "$INIT" "$INITIALIZED" "$LIST" | saya mcp serve --profile
  demo` (the `initialize`, `initialized`, and `tools/list` frames) — gets no
  reply to the frames still in flight at EOF. Hold stdin open until the
  replies arrive (`{ printf …; sleep 2; } | saya mcp serve …`); a host keeps
  stdin open for the session's life and every call is answered.

### Connecting a client

Add saya to the host's MCP configuration, with `--profile` values to fix the
allowlist (and `--allow-data-sharing` only when rows may leave the machine):

**opencode** — an `mcp` block in `opencode.json`:

```json
{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "saya": {
      "type": "local",
      "command": ["saya", "mcp", "serve", "--profile", "analytics"]
    }
  }
}
```

**codex** — in `~/.codex/config.toml`:

```toml
[mcp_servers.saya]
command = "saya"
args = ["mcp", "serve", "--profile", "analytics"]
```

or as one-off CLI overrides:

```bash
codex -c mcp_servers.saya.command='"saya"' \
      -c 'mcp_servers.saya.args=["mcp","serve","--profile","analytics"]'
```

**Claude Code**:

```bash
claude mcp add saya -- saya mcp serve --profile analytics
```

→ [ADR 0008](adr-0008-mcp-server.md) records the design and the client
validation.

`/export [--snapshot|--refresh] [--overwrite] <path>` writes rows to a
`.csv` or `.json` file (chosen by extension):

- The default form — and `--refresh` — re-runs the last query on its
  original connection and exports that **fresh read**; the file reflects the
  fresh result, not the displayed table (column filters, scroll, and folds
  do not apply). The legacy spelling stays a refresh and its success line
  says so.
- `--snapshot` writes the result you already inspected: the latest capture,
  held for this session only, with **no query at all**. The success line
  names the execution id and capture time (UTC) it came from. Captures
  cover three paths: a direct `/sql` result (full, up to the row cap), an
  agent `bounded_sql_query` result (model-limited: exactly the at most 50
  rows the agent saw — the success line says `model-limited: the full
  result may be larger — use --refresh for a full read`), and a saved
  replay run in the TUI (full).
- An existing destination needs `--overwrite`; a refused or failed export
  leaves the destination unchanged. Writes are atomic (private temp +
  rename), symlink and directory destinations are refused, and encoded
  output is capped at 32 MiB.
- A refused or absent capture never falls back to an older result: the
  snapshot says the rows were not captured and offers `/export --refresh`.
  Fan-out agent queries (`bounded_sql_query_all`) are never captured.

`/report [--rows N] [--overwrite] <path>` writes a shareable Markdown report
of the captured result — the latest capture, from a direct `/sql` query, an
agent query, or a saved replay — never querying a database and never opening
a browser or uploading anything. By default it carries the exact SQL and
provenance only (connection label, submitted-SQL hash, execution id, times
in UTC, row counts, truncation, scope) with the rows section labelled as
omitted; `--rows N` (at most 100) adds a table of the first N captured
rows, and a model-limited capture says so in the provenance (the agent saw
only the first N rows). Cell values are neutralised — links, images, HTML,
table-breaking characters, and control characters cannot be carried into
the file — and a bare `https://…`
text value may still be auto-linked by a Markdown renderer, so review the
report before sharing. The report is capped at 2 MiB, written atomically,
and an existing destination needs `--overwrite`. Use `/export` for data
files (`.csv`/`.json`).

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

`saya run` never prompts per tool call, whatever the approval mode: a run has
no per-call question. Scopes must be declared up front with `--allow <scopes>`;
a run without `--allow` refuses to start with exit `2` and creates nothing,
and `--approval-mode bypass` refuses at start too — a run's approval is its
`--allow` scopes; bypass is a session mode.
`--allow none` states the empty scope set — a deliberately read-only run: no
capability is approved, and the episode's per-tool-call approval stays at its
read-only default (read-shaped tools run, side-effecting tools are denied).

The plan is the one decision point. The model proposes the ordered steps; the
bound plan is persisted before you are asked, so a refusal or a crash leaves
a resumable record either way. When the command is attached to a terminal,
`saya run` asks exactly once: the prompt shows the goal, the scopes `--allow`
granted, each step with the scopes it asks for, the declared budgets, and the
workspace artifacts' digests as they stand. Only an explicit `y`/`yes`
approves; a refusal exits `2`, and that run is not resumable — nothing was
approved, so start a new run. Approve, and the episodes run without further
prompts: the wall clock arms only at approval, read-shaped tools run, the
approved scopes' own plan-gated tools run in the steps that asked for them
(scratch's `scratch_sql`, fetch's `http_fetch` and `http_download`, the
runner's `run_program`), and
anything needing an interactive decision or an
external side effect is denied rather than asked about. One approval instead of one per tool call is what
makes a long run usable and also what makes the approval matter, so the
residual is stated here rather than buried: if users rubber-stamp plans, the
security story leans on the sandbox, the bounds, and the sentinel tests.
Headless — piped input, CI, or `--non-interactive` — there is no ask at all:
the `--allow` declaration is the approval, and a plan asking for scopes
outside it is refused with exit `2`, naming the missing scopes.

**Today four scopes bind: `workspace-write`, `scratch`, `fetch` and
`runner`.**
`scratch` gives the run one DuckDB file of its own, at
`runs/<id>/scratch.duckdb`, reachable only through the `scratch_sql` tool, and
only in the steps whose plan asked for scratch: DDL, DML and joins over the
run's staged intermediate results, one statement per call, results capped at
50 rows. It holds nothing else: external access is off and locked at open,
every file reader is refused, and it is not a connection to any registered
database — the run's only writable SQL, and it dies with the run directory.
Stage corpus data through the workspace tools first. `fetch:<scheme>+<host>`
gives the steps that asked for it two tools, gated by that scope's declared
destinations and HTTPS-only, loopback/private/refused — `http_fetch` delivers
one bounded GET's body into the model's context as a labelled, escaped,
untrusted block (never raw bytes, never the system prompt), and
`http_download` streams one file into the run workspace under the run's
shared download budget: a tripped bound pauses the run fail-safe (`exit 6`),
leaving a resumable partial. Note the destination list is the *only* network
egress a run has: hosts outside it are refused, and every hop of a redirect
is re-judged. `runner:<program>` gives the steps that asked for it one tool,
`run_program`: one allowlisted program with typed argv — no shell, no
interpolation, one argv element per argument, ever. The programs a run may
name must be declared in `[jobs.runner]` (`allow` plus the absolute
`program_dir` they are staged in — see `docs/configuration.md`) and staged
there as regular, non-symlink, non-script files before the run; the
directory must sit outside the run's filesystem roots in both directions,
and the run refuses to start otherwise. A refused interpreter name is
refused by the grammar itself: `--allow runner:python3` is a usage error
(exit `2`) naming `interpreter:python3` as the family that approves it,
refused before any run directory exists — so the admission check below
keeps the refusal list in force for anything that still reaches it.

The runner is admitted only where the startup sandbox probe proved this
host. On a host the probe refused, a plan asking for the runner is refused
as needs-approval naming the missing scope — the honest refusal — because
the capability is absent, not degraded. On a proven host, every program the
run approved is checked once, at start: a program missing from the
directory, staged as a symlink or a script, not declared in
`[jobs.runner] allow`, or naming a refused interpreter refuses the run
(exit `3`) naming the program, the directory, and the reason. The refusal
list stays in force — shells and interpreters (`bash`, `python3`, …) can
spawn arbitrary children and are refused whatever any allowlist says; what
is stageable is a purpose-built, single-command binary. Each step can call
only the programs its own plan asked for: a step scoped to one program
rejects another allowlisted one.

The grammar also parses `endpoint:<role>=<endpoint>`, and it is **refused
with a usage error** that names what is missing, because no tool in a run's
universe consumes it yet. It is refused rather than accepted-and-ignored on
purpose: approving a capability that gates nothing would tell you the model
may do something it cannot. It becomes available with the slice that wires
it — per-step roles are not bound yet, every episode calls the orchestrator
endpoint, and this document does not describe a capability a run cannot
reach.

The grammar also parses `sql:<connection>`, and it is wired on both surfaces
now that headless runs sit on the same approval engine as interactive
sessions. On a run, the stated token seeds the run's decider — the frozen
session policy built from `--allow` — so under `--approval-mode ask` the
read-shaped SQL tools' calls that name that connection run without asking,
and every other ask the seeds do not cover denies with the engine's own
reason ("cannot prompt: a headless run's approval is its `--allow` scopes").
The token builds no plan capability — it is a per-call grant word — and the
run's journal carries it in the `PlanApproved` payload, so a resume
re-derives the grant from the journal, never from an editable file. The
scope is a session's word too — `/allow sql:<connection>` in an interactive
session pre-answers the same asks for the session's lifetime.

Interactive sessions grant these words without a run: `/allow <scopes>`
seeds the session's grant store through the same grammar judged for the
session surface (it accepts `sql:<connection>` and refuses
`endpoint:<role>=<endpoint>` — a session binds no per-step endpoint roles),
and `/grants` lists the store's tokens verbatim, one per line, sorted, under
a header stating the lifetime. A scope naming a denied program refuses at
`/allow` parse: a grant cannot override the deny list. The deny list bounds
the direct ask only — a denied `curl` does not stop an allowed `make` from
invoking curl, nor a renamed copy (`mycurl`) asked under its own spelling:
deny matches the exact program name named in the ask. Under `bypass`, `/grants` states the mode
first — "mode bypass: every call runs without asking; grants are not
consulted" — before the listing, because a token count alone would read as
"nothing runs" when the truth is that everything does. Grants die with the
session: they are never
persisted, and a resumed session starts empty. `/allow none` states the
empty approval and seeds nothing — it is not a revoke.

## The fourth approval mode: `bypass`

`--approval-mode bypass` (or `/approvals bypass`) runs every tool call
without asking: one typed, global, informed consent, given once at the flag
instead of once per call, in the session's own vocabulary — never a euphemism
and never a softened word. It is a **consent transformation, not a
containment transformation**: every structural guard still applies, unchanged,
under every mode. The SQL safety layer still refuses any write statement (a
refusal from the safety layer, never an approval question); the sandbox, the
placement guard, and the startup probe still gate `run_program`; path-shaped
names, symlinks, and scripts still refuse; fetches stay HTTPS-only and
private-range-refused; write-shaped tools still only appear where a workspace
root binds; and the all-bounds discipline is untouched. Bypass changes only
who answers the per-call question — and the answer is always yes.

What bypass opens honestly: the write-shaped session tools (`workspace_write`,
`scratch_sql`, `http_fetch`, `http_download`) are advertised whether or not a
prompt surface exists — a piped REPL runs under bypass too — and the session's
interpreter door opens to the interpreters the trusted config staged in
`[jobs.interpreter] allow` (the project layer is untrusted without
`--trust-project-config`; the door is the same staged universe under every
mode — under `ask` it is reachable through a granted
`interpreter:<program>` token, under bypass without the ask). An interpreter
child is confined by the same sandbox as any runner child: same fs roots,
same empty egress — and the process-fork fact is the platform's own: on
macOS the Seatbelt profile denies fork by omission, so a child an
interpreter spawns is refused by the sandbox; on Linux nothing in the
Landlock + namespace confinement restricts fork, so children run, under
the same bounds as the interpreter itself. What bypass does **not** open,
said plainly: interpreters outside the staged allow, programs outside
`[jobs.runner] allow`, unproven hosts (no runner in any mode — and the
absence is said: "run_program is unavailable: the sandbox probe did not
prove this host"), and write SQL.

When bypass takes effect — at launch, at `/approvals bypass`, and again on a
resume — the session prints its activation line: `bypass on: every tool call
runs without asking; every structural guard still applies.` With interpreters
staged, the line carries the interpreter warning in the session's wording
(the staged names, the sandbox bounds, the model-written program, and the
platform's process-fork fact — on macOS "no process-fork is granted:
children an interpreter spawns are refused by the sandbox", on Linux
"nothing in the Linux confinement restricts process-fork: children an
interpreter spawns run, under the same bounds as the interpreter itself");
with none staged, it says instead: `no interpreters are staged in
[jobs.interpreter] allow, so interpreter calls still refuse.` Where the
host-command lane composed, the line also states
`host commands run unsandboxed: as your user, your network, your filesystem.`
Where the session's deny list is non-empty, the line lists the denied names
(`denied for this session: <names>`); an empty list adds no line. What still
refuses under bypass: the lane when not composed, no workspace bound,
path-shaped and traversal names, names not on the passed PATH, timeout 0 or
over the ceiling, and programs on the session deny list. The status bar
and the headless status line render `approval:bypass` in red on every
surface. `/approvals ask` leaves bypass mid-session, effective for turns
started after the change; grants made before the toggle ride it and are
consulted again under `ask` — bypass consults no grant and records none.

A run never takes bypass: `saya run --approval-mode bypass` refuses at start
(`2`), and `saya run resume <id> --approval-mode bypass` refuses the same way
at the resume — a run's approval is its `--allow` scopes, and bypass is a
session mode. A `/run` from a bypass session therefore forwards no mode to
the child:
the child states its own scopes or takes the run default (read-only).

Budgets come from `[jobs]` in the config, layered with `--budget KEY=VALUE`
(`wall-clock=<seconds>`, `turns=<n>`, `tool-calls=<n>`,
`tokens.orchestrator=<n>`); a zero ceiling is refused as a typo, and the
environment is never read for budgets — a run is reproducible from its spec
and config.

Enforcement differs by dimension, and the difference is worth knowing.
`wall-clock` and `tokens` are checked as the run streams and **pause** it
(`BudgetExhausted` / `WallClockExceeded`, exit `6`, resumable) — and so does
the download budget a fetch scope implies: a download claim refused by the
run's wallet (1 GiB by default) trips a latch the engine checks each event,
pausing with the same `BudgetExhausted`, the partial left resumable. The
wallet's spend is journaled as a running level while the run claims it —
the durable record a resume carries, as usage is for the token ceiling.
`turns` and
`tool-calls` bound each episode through the agent's own limits, so exhausting
them ends the step rather than pausing the run. Because every episode calls
the single `orchestrator` endpoint today, `tokens.orchestrator` is the only
token ceiling a run accepts: a `tokens.<role>` key naming any other role —
from `--budget` or a leftover `[jobs] tokens_per_endpoint` entry — is refused
at start, because the engine binds the tightest declared ceiling to the
endpoint every episode calls, and a ceiling for a role that never runs would
silently cap the whole run. When per-step roles bind, attribution follows
the call.

Usage is journaled per provider call as the run spends it, and `saya run show`
renders the per-endpoint totals the journal recorded. A figure no call
reported renders `unknown`, never `0` — a provider that said nothing about its
cache is shown as `cache reads unknown`, while a provider that reported a
cache hit of zero shows `cache reads 0`. The usage shown for a run paused on
its token budget is the same arithmetic the ceiling compared (input plus
output), so the display and the budget that stopped the run agree.

**Two of the three streaming budgets bind the run, not each invocation; the
third re-arms.** The token ceiling carries: a run that pauses on its token
budget resumes against the same ceiling, because the resumed run's totals
are seeded from the usage its journal already records — so it pauses again
at the run's cumulative spend, and resuming without raising the ceiling
trips on the first tick, because the run is already past it. The download
budget carries the same way: the wallet a resume arms is seeded from the
download spend the journal records, so ten resumes share one wallet instead
of each spending the declared budget again. The wallet's check is the
refusal latch — the record of a claim that was refused, never a threshold
comparison — so a run already past its limit is refused on its next
download claim, and that refusal is what pauses it; a carried level never
stands in for a refusal no download made, and the headroom a record leaves
is claimable until a claim is refused. The wall clock, which no record can
replay, re-arms in full on a resume. Per-tool-call approval
defaults to `read-only` (read-shaped tools run,
side-effecting tools are denied). `--approval-mode ask` or `never` denies
rather than prompts — a run has no per-call question, so every tool that
declares it needs approval, the SQL tools included, is refused, while tools
that declare no approval need (schema discovery, the workspace reads) still
run. The episodes call the `orchestrator` endpoint from `[[ai.endpoints]]`.

Management subcommands:

- `saya run list` — every run, most recent first, with status.
- `saya run show <id>` — one run's status, goal, scopes, pause reason, the
  deliverables its steps recorded (sizes and digests), and the per-endpoint
  usage its journal recorded.
- `saya run log <id>` — the run's journal, one event per line; text mode
  renders what happened, `--format ndjson`/`json` emit the journal's own
  bytes.
- `saya run resume <id>` — continue a paused or crashed run at its first
  incomplete step. A run with a live holder refuses.
- `saya run cancel <id>` — record a run cancelled. A run with a live holder
  refuses; cancel the owning process with Ctrl-C instead.

Exit codes follow the global scheme, plus `6` for a paused run: a run that
stopped incomplete-but-not-failed exits `6` and says how to resume. A run
completing with failures exits by cause — safety/query `4`, provider/agent
`5`, connection/config `3` — never silently `0`. Ctrl-C cancels and exits
`130`; every refusal before or at the approval gate — unstated scopes, a
scope or budget the grammar refuses, a plan `--allow` did not grant, a
refused plan — exits `2`.
