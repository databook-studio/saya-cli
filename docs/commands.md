# Commands

Running `saya` without a subcommand starts the scrollback-preserving terminal
session. It accepts `/help`, `/connect`, `/connections`, `/include`,
`/exclude`, `/provider`, `/model`, `/privacy`, `/approvals`, `/allow`,
`/grants`, `/schema`,
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
and `--verbose`. `--workspace <dir>` (the interactive session only) binds the
session's workspace root explicitly; without it the root is the git worktree
top above the launch directory, and outside any worktree nothing binds.

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
workspace root, and privacy/cloud data-sharing state) followed by the
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
reach them.

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

`saya run` never prompts per tool call, whatever the approval mode: a run has
no per-call question. Scopes must be declared up front with `--allow <scopes>`;
a run without `--allow` refuses to start with exit `2` and creates nothing.
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

The grammar also parses `sql:<connection>`, and a run **refuses it with a
usage error** for the same reason: a run's decider consults no session
grant, so the scope would gate nothing. The scope is a session's word —
`/allow sql:<connection>` in an interactive session pre-answers the
read-shaped SQL tools' asks against that connection for the session's
lifetime — and headless runs are put on that same engine by a later wiring
item (U4). Until then a run states what it can act on, and `sql:` is not
one of them.

Interactive sessions grant these words without a run: `/allow <scopes>`
seeds the session's grant store through the same grammar judged for the
session surface (it accepts `sql:<connection>` and refuses
`endpoint:<role>=<endpoint>` — a session binds no per-step endpoint roles),
and `/grants` lists the store's tokens verbatim, one per line, sorted, under
a header stating the lifetime. Grants die with the session: they are never
persisted, and a resumed session starts empty. `/allow none` states the
empty approval and seeds nothing — it is not a revoke.

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
