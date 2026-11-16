# Configuration

SAYA uses TOML for non-secret settings and connection profiles. The CLI layer
looks for project files in `.saya/` and user files in the platform config
directory (`$XDG_CONFIG_HOME/saya`, `$APPDATA/saya`, or `~/.config/saya`).
Explicit `--config` and `--connections` paths override discovered files.

Run `saya config init` to create a safe starting pair — `config.toml` and
`connections.toml` — in your user config directory, the layer saya trusts. Pass
`--project` to write the `.saya/` pair in the current directory instead, for
settings a repository shares. The command never overwrites
existing files, writes `0600` files in a newly created Unix `0700` directory,
and makes a best-effort rollback if ordinary creation of the second file fails;
it is not crash-atomic. Text, JSON, and
NDJSON success output is stable; diagnostics stay on stderr.

Value precedence, highest first:

1. CLI flags
2. process environment
3. values from an explicitly supplied `--env-file`
4. project TOML
5. user TOML
6. built-in defaults

The process environment wins over the explicit env-file. `.env` is not loaded
implicitly. This makes CI and scripts predictable:

```bash
saya --env-file .env.saya config show
```

Profile selection is `--profile`, `SAYA_PROFILE`, `default_profile`, a sole
profile, then an error when multiple profiles exist. Environment-only mode is
supported by `SAYA_DB_TYPE`, `SAYA_DB_HOST`, `SAYA_DB_PORT`, `SAYA_DB_NAME`,
`SAYA_DB_USER`, `SAYA_DB_PASSWORD`, and optional `SAYA_DB_SSLMODE`. For MySQL,
`SAYA_DB_SSL_CA` is a SecretRef to PEM content and `SAYA_DB_SSLMODE` accepts
`disable`, `prefer`, `require`, `verify-ca`, or `verify-identity`; the safe
default is `verify-identity`. Password and CA values are retained only as
environment references in the typed profile. Use `disable` only for a local
TLS-disabled development server.

Environment-only Snowflake uses `SAYA_DB_TYPE=snowflake`,
`SAYA_DB_ACCOUNT`, `SAYA_DB_USER`, and `SAYA_DB_AUTH_TYPE=keypair|userpass|externalbrowser`.
Keypair additionally requires `SAYA_DB_PRIVATE_KEY`; userpass requires
`SAYA_DB_PASSWORD`; externalbrowser needs no secret. Optional context fields
are `SAYA_DB_WAREHOUSE`, `SAYA_DB_NAME`, `SAYA_DB_SCHEMA`, and `SAYA_DB_ROLE`.
Secret environment values become `SecretRef::Env` references and are never
placed in the resolved typed profile. Use an explicit `--env-file` or process
environment; process environment has precedence. File SecretRef paths are
literal and do not expand `~` or `$VARS`. `env` and `file` are supported;
keyring references are reserved and currently unavailable.

Environment-only SQLite uses `SAYA_DB_TYPE=sqlite` and `SAYA_DB_PATH`, with optional
`SAYA_DB_READ_ONLY` (defaults to `true`). `:memory:` is unsupported. SAYA
enforces bounded read-only SQL regardless of the setting.

The `[run]` table sets the query policy and result caps:

```toml
[run]
read_only = true            # SAYA_READ_ONLY overrides this
max_rows = 1000
query_timeout_seconds = 60
```

`read_only` (overridable by `SAYA_READ_ONLY`) is the global SQL/query policy and
also drives session-level read-only on connectors that support it. This is
distinct from a profile's own `SAYA_DB_READ_ONLY`, which sets SQLite's file
access mode.

The `[limits.run]` table sets the default budgets a `saya run` is declared with
when its specification and each of its steps declare none. Each key is optional
and independent; the run pauses when a declared budget trips rather than
silently stopping.

```toml
[limits.run]
turns = 40                # per-episode turn ceiling
tool_calls = 25           # per-episode tool-call ceiling
wall_clock_seconds = 1800 # run wall-clock ceiling
no_progress_turns = 8     # stop a step after this many idle turns (0 disables)
repeat_limit = 3          # stop after this many identical results (0 disables)

[limits.run.tokens_per_endpoint]
"local-ollama" = 200_000  # token ceiling per named endpoint
```

`no_progress_turns` and `repeat_limit` are progress guards, not budgets: they
stop a step whose loop is not getting anywhere and pause the run, after the
model has taken one final turn to write its best state. A step that hits one
pauses the run rather than being recorded complete.

Each dimension is a positive integer or the exact string `"unlimited"`.
`turns` has no default: unset means no ceiling, like `wall_clock_seconds`
and `tool_calls` — a ceiling left unset is unlimited at the contract level,
and the run pauses when a declared budget trips rather than overrunning.
`wall_clock_seconds` measures first-party elapsed wall time from approved
execution, including pauses and time between resume invocations. Its origin
and observed high-water mark live in the run journal. A configured resume
refuses when a legacy journal has no valid origin; start a newly approved
run to obtain a new clock. Outside forced boundaries, raised observations
persist at a one-second cadence; writes are forced at approval, resume,
provider turns, tool requests, and lifecycle transitions. Forward clock jumps
may expire early, and
backward changes are refused when an observation detects them. Changes
between observations may not be detectable. This is not CPU time; a separate
timer for stalled operations is planned independently. `tokens_per_endpoint` is
keyed by run-scoped endpoint name (the same shape `[[ai.endpoints]]` uses);
more than eight keys, or a key outside the name shape, is a rejected config.
With several ceilings declared, the tightest binds today.

A zero on any budget is refused as a typo rather than clamped — zero turns or
zero tool calls would pause a run before its first turn — with a typed error
naming the field. There is no upper bound; the unlimited case is "leave it
unset".

`[jobs.fetch]` sets the download budgets the `http_download` tool spends
from. Every key is optional and resolves to a conservative default, so a run
is download-bounded even when nothing is declared:

```toml
[jobs.fetch]
max_file_bytes = 268435456   # per file; default 256 MiB
max_run_bytes = 1073741824   # whole run; default 1 GiB
timeout_seconds = 60         # per request; default 60
```

A zero there is refused the same way. No `[limits.run]` or `[jobs.fetch]` key
has an environment override, deliberately: a run must be reproducible from its
specification and config alone. Per-invocation overrides belong on the command
line instead — `saya run --budget turns=40 --budget
tokens.orchestrator=100000` — whose known keys are `wall-clock=<seconds>`,
`turns=<n>`, `tool-calls=<n>`, and `tokens.orchestrator=<n>`: today every
episode calls the orchestrator endpoint, so a `tokens.<role>` ceiling naming
any other endpoint is refused at start (the map's shape is still validated
here, at config resolve time). Unset keys fall back to `[limits.run]`, and a
zero is refused as a typo there too.

## Limits at a glance

Every agent surface is bounded, and each bound is stated in config rather than
hidden in the binary. `saya config doctor` prints the resolved value of every
key below.

| Surface | Table | Defaults |
| --- | --- | --- |
| Interactive session | `[limits.session]` | 24 requests, 64 tool calls, 300 s, 250,000 known tokens |
| `saya ask` | `[limits.ask]` | every unset key falls back to `[limits.session]` |
| `saya run` | `[limits.run]` | `turns`, `tool_calls` and `wall_clock_seconds` unset (no ceiling); `tokens_per_endpoint` unset |
| Job | `[limits.job]` | bounded: 80 logical requests, 240 transport attempts, 300 tool calls, 400,000 known tokens per loop, 1,200 s; no job-wide token ceiling |

A run pauses when a declared budget trips rather than silently stopping. A zero
is refused as a typo, not clamped. `[limits.job] budget_mode = "uncapped"`
removes the job's aggregate ceilings; concurrency, child, command, per-response
output, context-size, timeout, authority, cancellation and SQL safety bounds
still apply.

`[job] program_dir` names the trusted, operator-owned directory the job host
and its extensions launch programs from:

```toml
[job]
program_dir = "/opt/saya-programs"   # unset by default
```

The path must be absolute — a relative one is a typed resolve error, because
the canonical form must not depend on the working directory the config was
loaded from. Existence is deliberately not checked at resolve time, so a
dangling path cannot break `saya ask` or `saya query`; a job that cannot
provision it fails closed instead. The whole `[job]` table is on the protected
list, so a project-layer config cannot set `program_dir`, or any other `[job]`
key — set it in your user config, or pass `--trust-project-config`. The table
holds `program_dir` and the extension programs (`extensions`,
`automatic_lsp_profile`); a job's ceilings are `[limits.job]` below, and
the helper lane is the top-level `[agents]` table.

`[job] root_mode` selects where a job works:

```toml
[job]
root_mode = "in_place"   # the default; "managed" creates a checkout
```

`in_place` edits the stated folder itself, including a Git working tree, and
records tool edits as before-images; `managed` creates a managed checkout from
a Git repository. `saya start --managed` overrides the configured default for
one job, and `--in-place` overrides it the other way. `saya follow-up` follows
the job's own record and ignores both flags.

The `[coding]` table was renamed to `[job]`, and its ceilings moved to
`[limits.job]`. A config that still sets `[coding]` fails to parse with a
message naming the section to move. The `[jobs.runner]` and `[jobs.interpreter]`
sections are gone, along with the `run_program` tool they configured;
`[job] program_dir` is where their `program_dir` moved. See
[Removed in 0.5.0](#removed-in-050).

The `runner:` and `interpreter:` scopes and grants are gone: a fresh plan,
spec, or session that names one is refused with a pointer to the `shell` tool.
A persisted plan or spec that names one still loads — the words are kept on
the wire — but is refused as needs-approval at approval rather than read as an
unknown scope.

The project layer may set `[limits.run]` and `[jobs.fetch]` without
`--trust-project-config`: they are cost controls, not security-critical
settings. Layering is per key: a layer that declares a key replaces that key's
whole value from the lower layers, so the `tokens_per_endpoint` map and the
`[jobs.fetch]` sub-table are replaced wholesale rather than merged field-wise.

`[host_commands]` shapes the interactive session's unsandboxed host lane,
which composes wherever a workspace root binds — no declaration needed.
User-layer only — a project-layer `[host_commands]` is a typed resolve
error, because a model-writable file must never shape unsandboxed execution:

```toml
[host_commands]
pass_env = ["CI_TOKEN"]    # parent variable names the built child env carries
timeout_seconds = 600      # per-call ceiling; a call may narrow, never widen
```

`run_command` claims no containment: the child runs as your user with your
whole filesystem and network, resolved on your PATH. The contained lane's
guarantees are the sandbox's, not this one's. Under bypass, a hostile
workspace file is effectively arbitrary code execution as the user.

`[session_commands] deny` states the session's deny list of bare program
names. User-layer only — a project-layer `[session_commands]` is a typed
resolve error — and refusal-only: it composes nothing, and gates every
session door that execs a program by name (`run_command`, and the `shell`
host lane), before every grant, every approval prompt, and bypass, in
every mode. The deny list bounds the direct ask only — a denied `curl` does
not stop an allowed `make` from invoking curl, nor a renamed copy (`mycurl`,
a symlink or copy of curl) asked under its own spelling: deny matches the
exact program name named in the ask, never content or resolved identity:

```toml
[session_commands]
deny = ["curl", "ssh"]
```

The `[ui]` table sets the interactive TUI's colour palette:

```toml
[ui]
theme = "auto"   # dark | light | auto
```

`theme` selects the palette the full-screen TUI paints with. `auto` (the
default) honours the `COLORFGBG` environment variable when the terminal
publishes it — a background of 7–15 selects the light palette, anything else
stays dark — and falls back to dark when `COLORFGBG` is absent or unparseable,
since a silent guess at the wrong theme is worse than the common case. The
`--theme <dark|light|auto>` global flag overrides this for a single
invocation, and the usual CLI-over-config precedence holds exactly: with no
`--theme` flag the configured `theme` applies; an explicit
`--theme auto|dark|light` wins over it, `auto` included; with neither, the
default `auto` applies.

The `[ai]` table tunes the provider request the agent loop assembles.
`context_byte_budget` is the ceiling on the approximate byte size of the whole
conversation sent to the provider (system prompt, user question, history, and
the tool results the loop appends). When unset and the model's context window
is known — declared in `context_window_tokens` or found in the built-in table —
the budget is derived from it as four bytes per token times 90%, clamped to a
256 KiB floor and a 4 MiB ceiling. An unknown window, or a window whose
derivation lands on the floor, keeps the historical 256 KiB default. An
explicitly set value always wins and is not clamped at the top. When the assembled
conversation grows past it, the loop **trims rather than aborts**: it drops the
oldest complete tool-result group first (the assistant turn that issued the
calls plus its tool messages), keeping the newest context; if only the newest
group remains and it alone is over budget, the largest tool result is truncated
in place with a visible `…[truncated]` marker so the model knows it saw a cut
result. The run never fails for reaching the budget. Raise it on a model with a
large context window to keep more history; a value below 1024 bytes is rejected
as too small to hold a single turn. It has no environment variable or CLI flag
of its own — set it in the `[ai]` table — and follows the usual file-then-defaults
precedence.

```toml
[ai]
context_byte_budget = 524288   # 512 KiB; default is 256 KiB
```

`request_timeout_seconds` is the total deadline for one model request, default
600 s. A stream still flowing at the deadline is cut and the partial reply is
kept; the next request starts a fresh deadline, and consecutive cuts are
counted separately from completed replies. The 600 s default is long enough
for a deep reasoning call while still bounding a stalled stream; raise it for
longer thinking calls.

`retry_delays_ms` is the provider retry schedule, in milliseconds and in
order. The default is eight delays — 500, 1000, 2000, 4000, 8000, 16000,
30000, 30000 — so a request that keeps failing is retried eight times with
exponential backoff. Every scheduled wait is jittered uniformly between half
the delay and the delay, so concurrent clients do not retry in lockstep. A
request is retried only on HTTP 429, any 5xx (which includes Anthropic's 529
overloaded response), and a connection reset, abort, broken pipe, or
unexpected EOF. Every other 4xx is the request's own fault — a wrong model,
key, or body — and is returned at once. A request whose stream already
delivered content is never retried: the partial answer stands and the cut is
reported. A `Retry-After` header in integer seconds replaces the scheduled
wait and is clamped to 60 s; a configured delay above 30 s is a configuration
error rather than a longer sleep. The whole
retry budget for one logical request is bounded by its
`request_timeout_seconds` deadline and by the job's remaining wall clock,
whichever is smaller, and stops with the typed deadline failure when either
expires. The list takes at most eight entries, each at most 30,000 ms; an
empty list disables retries. The idle timeout is a separate bound and never
triggers a retry.

```toml
[ai]
retry_delays_ms = [500, 1000, 2000, 4000, 8000, 16000, 30000, 30000]
```

`max_output_tokens` sets the reply budget: the provider is asked to cap its
completion at it, and it doubles as the output reservation subtracted from a
known context window during request preflight. It is unset by default: no limit
is sent unless you configure one, and each provider falls back to its own
default (Anthropic, whose API requires `max_tokens`, uses 32,000). A value of
`0` is rejected as a typed configuration error rather than treated as unset.

`reasoning_effort` asks the model to think harder — or less hard — before it
answers. It is **adaptive by default**: unset (or an explicit `"adaptive"`)
means the loop chooses per request. The first request of a run asks for
`high`, and so does a request whose immediately preceding tool results contain
a failure — a shell `exit_code` other than `0`, a `timed_out` result, a tool
error or refusal, or non-empty post-edit diagnostics. Every other request asks
for `low`, so routine turns stay cheap while the two moments that need
judgement think hard.

Set `low`, `medium`, or `high` to pin every request to that effort instead.
Each provider spells the effort its own way:

| provider | wire |
| --- | --- |
| `openai`, `openai_compatible` | `reasoning_effort: "low" \| "medium" \| "high"` |
| `openrouter.ai` | `reasoning: {effort: "low" \| "medium" \| "high"}` |
| `anthropic` | `high` is adaptive thinking (`thinking: {type: adaptive}` plus `output_config.effort` and the `effort-2025-11-24` beta header); `low`/`medium` are thinking token budgets |
| `gemini` | a `thinkingBudget` of 1,024 / 4,096 / 8,192 tokens |
| `ollama` | its `think` boolean (`low` off, `medium` and `high` on) |

An endpoint that supports no effort field receives none, whatever the policy
chooses. `minimal` and `default` are **not** accepted. `minimal` is the one
spelling outside `{adaptive, low, medium, high}` that the OpenAI wire has, and
some gateways reject it with HTTP 400 — a setting whose value can fail every
request is not a setting worth offering; `default` would be a second name for
adaptive. An unaccepted value is a typed configuration error that names the
accepted ones.

The setting reaches the agent loop's own requests. A call that states an effort
for itself — the post-turn extraction call asks for `minimal` — is never
overridden by it. Whether the model *complied* is only visible in the reported
reasoning tokens (`/usage`), never assumed: saya reports what it asked for.

```toml
[ai]
reasoning_effort = "adaptive"   # adaptive | low | medium | high
```

`compaction` chooses how context compaction behaves once the window fills:
`auto` (the default; summarise on crossing the compact threshold), `manual`
(only an explicit `/compact`), or `off` (no automatic trigger and no context
warning). `SAYA_COMPACTION` overrides it in the environment.

`compaction_trigger_percent` is the share of the known context window, in
percent, at which the agent loop compacts the conversation (default `50`). The
token estimate is compared against that fraction of the window, so a large
window compacts long before the byte budget fills. The window comes from
`context_window_tokens` when it is declared, or from the built-in table when the
model is known; with no known window the byte budget's own trigger still
applies. `context_byte_budget` remains the hard ceiling in every case — this
setting can lower the compaction point but never raise the bytes sent. Values
outside `1..=100` are rejected.

```toml
[ai]
compaction_trigger_percent = 50   # 1..=100; default is 50
```

`app_attribution` controls OpenRouter's app-attribution headers. Every model
request carries `User-Agent: saya-cli/<version>`; when the base URL's host is
`openrouter.ai`, Saya also sends `HTTP-Referer:
https://github.com/databook-studio/saya-cli` and `X-Title: Saya CLI`, so
OpenRouter's Top Apps lists Saya by name. It is on by default; set
`app_attribution = false` (or `SAYA_APP_ATTRIBUTION=false`) to omit the two
OpenRouter headers. They are never sent to any other host.

`context_window_tokens` declares the model's context window in tokens. saya
keeps a small built-in table of published models — GLM, GPT, Claude, Gemini,
and the common Ollama families — and looks a model up by exact name, so
`glm-5.2` and `qwen2.5-coder:14b` resolve to their documented windows without
any configuration. A model the table does not know stays **unknown**: most
saya users run through a gateway serving models the table will never list, and
guessing a window for one would either refuse work that would have succeeded or
promise headroom that does not exist. A known window is enforced at request
preflight: each provider request is measured against the window minus the
output reservation (`max_output_tokens` above) minus 1,024 tokens of slack,
and an oversized request first sheds the oldest history and trims remaining
content before the turn is refused with a typed context-window budget error.
An unknown window keeps the request capped by `context_byte_budget` alone.

For a gateway model the table will never hear of, declare the window yourself —
a declared value wins over the table, because it says something about *your*
endpoint that a published fact for the model name cannot:

```toml
[ai]
model = "my-gateway-model"
context_window_tokens = 1048576
```

A declared value of `0` is rejected as a typo. There is no upper bound.

Interactive investigations and the one-shot `ask` surface resolve four budget
ceilings from `[limits.session]`: `requests` (24), `tool_calls` (64),
`elapsed_seconds` (300), and `known_tokens` (250,000). `[limits.ask]` may state
its own value for any of the four; every key it leaves unset falls back to
`[limits.session]`. These are conservative starting policy values, not
production percentile measurements. The elapsed ceiling is an active-time
wallet rather than a wall-clock total: it starts before connector setup, stops
before post-turn learning, and only active answering segments debit it, so
ancillary work neither consumes the wallet nor is cancelled by it. One wallet
serves every answering invocation for the question — candidates and
clarification replies included. Each accepts a positive integer or the exact
string `"unlimited"`; zero, negative values, other strings, and non-integer TOML
values are errors. Integer literals beyond TOML's signed 64-bit range are
invalid TOML and fail while parsing. `[limits.session]` and `[limits.ask]` are
protected: a repository project config cannot change these trusted budgets
unless `--trust-project-config` is supplied. No environment aliases are defined
for them. Enforcement is live: each answering invocation admits against these
limits, and a crossed ceiling stops the run with a typed budget stop reason
naming the dimension — requests, tool calls, reported tokens, or elapsed time.

```toml
[limits.session]
requests = 24
tool_calls = 64
elapsed_seconds = 300
known_tokens = 250000

[limits.ask]
tool_calls = 32   # every unset key falls back to [limits.session]
```

The `[[ai.endpoints]]` array declares the named endpoints a run's roles can
bind to (`saya run --allow endpoint:<role>=<endpoint>`). Each entry is a delta
over the plain `[ai]` block: a field the entry declares wins, an unset field
inherits `[ai]`'s resolved value, and `name` never inherits. The resolved pool
is keyed by name and always contains `orchestrator` — the plain `[ai]` block
when no entry carries that name — so a config without the section changes
nothing.

```toml
[ai]
model = "qwen2.5-coder:14b"
api_key = { env = "SAYA_API_KEY" }

[[ai.endpoints]]
name = "planner"
base_url = "https://gateway.internal/v1"
api_key = { env = "SAYA_PLANNER_KEY" }
```

`name` is required and must have the run-scoped name shape: non-empty, at most
128 characters, no whitespace or control characters. Two entries with the same
name in one file are a typed error, not last-wins; more than eight entries are
rejected; an unknown key inside an entry is a parse error naming the key.
Across layers, an entry whose name a trusted layer already declared overlays
that endpoint field-wise — an absent field leaves the lower layer's value.
Declaring an entry named `orchestrator` replaces the fallback. Endpoints have
no environment variable or CLI flag of their own.

`api_key` is a secret and must be a reference — `{ env = "SAYA_VAR" }` or
`{ file = "..." }` — never an inline value. An inline string fails to parse
with a diagnostic that names the endpoint (`ai.endpoints["planner"].api_key`),
not just the section.

The project layer's `.saya/config.toml` is a security boundary here, and so is
a file supplied with `--config`, which occupies the same untrusted slot.
Without `--trust-project-config` (or `SAYA_TRUST_PROJECT_CONFIG`) the project
layer cannot add an endpoint name the trusted layers never declared — the
*name set* is protected, because an endpoint they never declared is still an
attacker-chosen destination — and it cannot change an existing endpoint's
`base_url` or `api_key`, the two fields that decide where a request goes and
which credential authenticates it. Reverted attempts are reported naming the
endpoint (`ai.endpoints["planner"].base_url`); every command prints a one-line
warning and `config doctor` lists which settings were ignored. An endpoint's
`provider` and `model` are ordinary settings, like `[ai] model`: they name
which model answers, not where the request goes.

## Model tool allow list

`[tools] allow` is one list that governs every agent surface — the interactive
session, `saya ask`, `saya run`, and jobs with their helpers:

```toml
[tools]
allow = ["read", "grep", "shell"]
```

The global `--tools name,name,...` flag overrides it. Only the surfaces with a
tool list read it — `saya`, `saya ask`, `saya run`, and `saya start` — and
every other subcommand refuses it by name rather than dropping it. Inside a
session, `/start --tools …` may only narrow the list the session itself
advertises: naming a tool the session does not have is an error, and omitting the
flag hands the job the session's list unchanged.

The list narrows what a surface composes, and it can re-advertise a tool the
lean default withholds; it never grants a tool whose own condition fails.
`shell` stays hidden under `--approval-mode never` however the list is spelled,
and a call to a tool that is not advertised is refused as an unknown tool.
With no setting at all, each surface advertises its own default. A job's
default is `read`, `write`, `edit`, `glob`, `grep`, `shell` and `checklist`,
plus `notebook_edit` where the checkout holds a notebook, `submit_plan` in plan
mode, the background shell readers after a background run, and configured MCP
tools. `list` and `delete` are composed but withheld by default, and
`web_fetch`, `web_search` and `helpers` appear only when the request or
configuration gives evidence for them. Naming a withheld tool in the list
advertises it again, so `allow = ["list", "delete"]` opts back in. Duplicates
are de-duplicated and `bash` names `shell`; an empty
list, a word that names no tool, or a database tool name is a typed error that
lists the valid names. So is `helpers` on its own: a helper runs the list minus
`helpers`, so a list of `helpers` alone would hand every helper nothing.

Database tools are **not** governed: `schema_discovery`, the bounded SQL family,
the query fan-out, and the contract tools follow the selected connection and the
data-sharing setting, not this list. `schema_discovery` needs only a selected
connection or profile; the other eight also need data sharing on
(`--allow-data-sharing`, `[ai] allow_data_sharing`, or `/privacy`), and the two
contract tools need a state store as well — so they stay hidden with a connection
selected when sharing is off. Naming one is refused with that reason rather than
silently ignored.

A job persists the set it started with, so a resume runs exactly what the
operator approved at start, and a narrowed set is shown in the start proposal.
A helper runs the job's set minus `helpers`: only the root delegates.

Tools from configured MCP servers *are* governed, in two spellings:
`mcp__<server>` keeps every tool of that server — including tools it had not
advertised when the list was written — and `mcp__<server>__<tool>` keeps exactly
that one. A `mcp__` word that is not one of those two shapes is refused at
launch by shape. See [MCP servers](#mcp-servers) below.

`[tools]` is protected like `[job]`: an untrusted project config cannot set
it, and a reverted attempt is reported as `tools`.

## Permissions

`[permissions]` holds the session's consent gates:

```toml
[permissions]
allow_bypass = true               # default: false
allow = ["cargo test", "git diff"]
deny = ["rm"]
```

`allow_bypass` decides whether a session may *enter* the `bypass` permission
mode — it does not enter it. With the key unset (the default), `/mode bypass`,
the Shift+Tab cycle and `--permission-mode bypass` all refuse, and a resumed
session whose record says `bypass` degrades to `ask`. With it true, all three
can reach the mode; entering it is still an explicit act, and the activation
line prints when it happens. See
[commands.md](commands.md#switching-the-sessions-permission-mode-mode-shifttab---permission-mode).

The key governs the session's mode switch only. A launch that states the
posture itself — `--approval-mode bypass` — is this process's own consent and
is unaffected. A `saya run` never takes bypass whatever this key says.

`allow` and `deny` are command *prefixes*, matched word by word: `cargo test`
covers `cargo test -p saya-config` and never `cargo testx`. At most 64 rules
per list, each at most 256 bytes and free of control characters; a refused
entry is a configuration diagnostic, not a startup failure.

- **`deny`** refuses the command in every mode, `bypass` included, and wins
  over any allow rule. The refusal names the rule. Deny is a prefix rule, not
  a sandbox: it matches the command as written, so `/bin/rm`, `command rm`
  and `\rm` are different prefixes. The one exception is an absolute program
  path, whose basename is matched — `/bin/rm -rf x` hits deny `rm`.
- **`allow`** lets a command take the host lane without asking under `ask`,
  with or without an operator. It never widens `read-only` or `never`.
- A compound command (`&&`, `||`, `;`, `|`, `|&`, a single `&` for a
  background job, a newline or carriage return) is allowed only when *every*
  segment matches an allow rule; a trailing separator leaves an empty segment
  that matches nothing.
- Output redirection is not covered by a prefix rule (`git diff > ~/.zshrc`,
  `cargo test > out.txt`, `&>`, `>>`, `>|`, `2>`), except a redirect to
  `/dev/null` and an fd duplication such as `2>&1` or `>&2`. Input
  redirection from a file (`< input.txt`) stays covered.
- A command containing a command substitution (`$(`, backticks), process
  substitution, or a heredoc is never auto-allowed: the matcher cannot see
  what it would run. The same holds for unbalanced quotes. A backslash-newline
  is a line continuation, not a segment separator.
- A leading `cd <dir> &&` is stripped only when the directory is inside the
  workspace (or relative with no `..`); `~`, `$...`, and `-` are not known to
  be inside it, so the `cd` segment must match a rule itself. A leading
  `FOO=bar` assignment is stripped for the deny side only: on the allow side
  it makes the segment match nothing.

`--allow-command <prefix>` and `--deny-command <prefix>` state the same rules
at launch, repeatably, on `saya start`, `saya run` and the interactive
session; the flags add to the configuration lists rather than replacing them.

`[permissions]` is protected whole, like `[tools]`: a project's
`.saya/config.toml` cannot set it, and an attempt is reverted and reported as
`permissions`. A repository must not be able to hand itself the one consent
that answers every per-call question — nor state the rules that pre-answer a
command.

The key is read once, when the session starts, and carried on the session for
its whole life: `/reload` reports a changed value as taking effect in a new
session.

## MCP servers

`[mcp_servers.<name>]` names a program whose tools join the agent's surface:

```toml
[mcp_servers.files]
command = "mcp-files"
args = ["--root", "."]
env = { FILES_TOKEN = { env = "FILES_TOKEN" } }
startup_timeout_ms = 5000
call_timeout_ms = 30000
```

The server speaks MCP over stdio, and each tool it advertises reaches the model
as `mcp__files__<tool>`. Off until configured. `env` values are secret
*references* (`{ env = "NAME" }` or `{ file = "path" }`), never literals — a
literal is refused with the reference form named. Remote (HTTP) servers are not
supported in this release: a table that states a `url` is refused with a message
naming `command`. Bounds: 8 servers, 64 tools per server, 1024 characters per
description (truncated, not refused), 16 KiB per input schema, 30 s for the
whole startup pass, and `startup_timeout_ms` per server (default 5000, ceiling
30000; `call_timeout_ms` defaults to 60000 with a ceiling of 600000).

A server's definitions are untrusted input: a tool whose name a provider's
function field cannot carry, whose schema is over the byte cap, or whose exposed
name collides with another of the same server is dropped rather than renamed,
and every drop is reported at session start. Every call follows the host
`shell`'s own consent rule — `bypass` runs it, `ask` puts each call to the
operator, `read-only` and `never` advertise no `mcp__` name at all — and the
answer is bounded and has the server's own `env` credentials scrubbed out.

`[mcp_servers]` is protected: a project-layer config cannot add a server,
replace one, or retarget one at a different program.

## Helper agents

`[agents]` states the helper lane a job may open: the two bounds its
scheduler runs under, and the helper types its `helpers` tool advertises, keyed
by the `subagent_type` a call sends.

```toml
[agents]
max_children = 3                     # helper children running at once
max_concurrent_model_requests = 4    # model requests in flight at once

[agents.reviewer]
role = "analyst"        # read-only; the default for a new type

[agents.builder]
role = "implementer"    # may write, inside the job's own ceiling
allow_write = false     # ...unless this narrows it back to read-only
model = "fast-model"    # ...and its helpers ask for this model
```

`max_children` (default `3`, `1..=3`) is how many helper children one job may
have running at once. `max_concurrent_model_requests` (default `4`, `2..=4`) is
how many model requests the job may have in flight at once, root and helpers
together, so it must admit at least the two a coordinator needs. Both are whole
numbers; anything outside its range is a typed resolve error naming the key.
`--agents-max-children` and `--agents-max-concurrent-model-requests` override
them for one invocation. The two names are reserved: a key in `[agents]` that is
not one of them is a helper type, and one of them written as a table is refused
rather than read as a type.

The two built-in types are `explore` (read-only) and `general` (implements one
piece); a config that declares no entry has exactly those. A declared entry may
only narrow a type that already exists: a new type starts read-only, and
`role = "implementer"` is what lets one write. A widening entry — `explore` as
an implementer, or `allow_write = true` on an analyst — is a typed resolve
error, not a silent no-op. A type is a shape, never a grant: each helper's
authority is still narrowed from the job's own scope, so a configured
implementer writes only where the job already could.

`model` gives a type's helpers a model of their own: they ask the job's own
endpoint for it, and without it they send exactly the model the job itself
sends. It selects what answers and never where the request goes — the provider,
base URL and credential stay the job's, so an entry cannot point a helper at
another provider — and it is bounded like `ai.model`. The name need not be one
of the built-in table's: a gateway serving a name of its own is served the same
way the root is. It never changes a helper's authority.

`[agents]` is protected: an untrusted project config can neither set either
bound nor add or retarget a type, and a reverted attempt is reported as
`agents`. It is a top-level table rather than a `[job]` key because a helper
type is not coding-only — the same shape a session or a run would advertise.

## Hooks

`[hooks]` runs your own commands at six points in an agent's life. Each event
is an array of tables:

```toml
[[hooks.pre_tool_use]]
matcher = "shell"          # a tool name, or "*" for every tool; omit for every tool
command = "./scripts/check-shell.sh"
timeout_ms = 5000          # default 10000

[[hooks.post_tool_use]]
command = ["/usr/local/bin/audit", "--call"]   # argv form: one word per element

[[hooks.permission_request]]
matcher = "shell"
command = "./scripts/check-ask.sh"

[[hooks.session_start]]
command = "cat .saya/session-context.md"

[[hooks.session_end]]
command = "notify-send 'saya session ended'"

[[hooks.stop]]
command = "notify-send 'saya finished'"
```

- **`pre_tool_use`** runs before a tool call. Exit `2` **blocks** the call, and
  the hook's stderr (bounded, redacted) is returned to the model as the refusal
  reason. Exit `0` allows it. Every other exit — any other code, a signal, a
  timeout, a command that could not start — allows the call and prints one
  warning line naming the hook.
- **`post_tool_use`** runs after a call. It observes; it decides nothing.
- **`permission_request`** runs when a call would **ask** you for approval,
  before the prompt is shown. Exit `2` denies the call, and the hook's stderr
  (bounded, redacted) is the reason the model reads. Exit `0` is **not** an
  approval: the normal prompt is still shown and still decides. A call the
  mode would not ask about runs no `permission_request` hook at all, so the
  hook can only narrow an ask, never pre-empt an automatic decision.
- **`session_start`** runs once when an interactive session or a job root
  starts. Its stdout, bounded and redacted like every other hook's, is offered
  to the model as additional context on every turn of that session or job. A
  non-zero exit is a diagnostic and offers no context.
- **`session_end`** runs best-effort when the session or job root ends, on a
  normal end and after a cancel alike. It observes; it decides nothing, and
  each hook is still bounded by its own timeout, so shutdown waits no longer
  than that.
- **`stop`** runs when the agent finishes a turn. It observes.

The hook reads one JSON object on stdin: `hook_event_name`, `tool_name`,
`tool_input` (the call's arguments as a redacted, bounded **string**), and
`tool_succeeded` for `post_tool_use`, plus the caller's ids — `job_id` for a
job's root, `job_id` and `agent_id` for one of that job's helpers,
`run_id` for a `saya run`, or `session_id` for an interactive session.
`session_start` and `session_end` carry `hook_event_name` and the caller's
ids only.

**A hook can only take a call away.** It cannot grant a tool, widen a scope,
change the approval mode, or answer an approval prompt on your behalf: a hook
that exits `0` for a tool the `[tools] allow` list removed, or for a tool the
mode hides, changes nothing — the surface still refuses that call, and
`permission_request` exit `0` changes nothing about the prompt that follows.
`read-only` and `never` keep no host lane, so they run no hook at all.

`pre_tool_use` and `post_tool_use` run on every surface that runs tools: the
interactive session, `saya ask`, a job — its root **and** every helper it
delegates to — and every step of a `saya run`. `permission_request` runs where
a surface can put a call to you: the interactive session and `saya ask`. A
headless job or run asks nobody, so it has no ask to pre-empt.
`session_start` and `session_end` run for an interactive session and a job
root. `stop` runs when any of those loops ends: the interactive session,
`saya ask`, a job's root and each of its helpers, and each step of a `saya run`
(a retried step fires it per attempt, because each attempt is its own loop).

A hook is your own command, so it runs on the host with the same environment
scrubbing a host `shell` call gets, through the same bounded capture and
process-group kill. Bounds: 8 hooks per event, an 8 KiB command, a 10 s default
timeout (60 s ceiling), an 8 KiB stdin payload, a 1 KiB reason returned to the
model, and 1 KiB of `session_start` context offered to the model. A hook that
hangs is killed at its timeout and never wedges the run.

`[hooks]` is protected: an untrusted project config cannot set it, and a
reverted attempt is reported as `hooks`.

## Job limits

A job's ceilings live in `[limits.job]`, which is protected whole.
`limits.job.budget_mode = "bounded"` is the default. The explicit trusted-user
setting `budget_mode = "uncapped"` removes configured aggregate request,
physical transport, tool-call, provider-reported-token, and elapsed-time
ceilings for jobs:

```toml
[limits.job]
budget_mode = "uncapped"
```

A healthy job is stopped by its progress guards, not by spending counts:
after `no_progress_turns = 8` consecutive model turns with no progress, or
after `repeat_limit = 3` identical results in a row from the same call (the
third repetition gets one note; the next stops the job). Progress is a
successful workspace write, edit, delete, or rewind; a changed test/build
outcome; the first successful read, grep, or glob of a path or pattern; a
checklist status change; or a plan submission. Re-reads, identical commands
with identical results, and text-only turns are not progress. A stop still
gives the model one final turn to write its best state and summary, and the
candidate is prepared as for a normal finish. Set a guard to `0` to disable it.

The request, transport, tool-call, and elapsed dimensions are runaway
backstops, not spend targets: 2,000 logical requests, 6,000 transport attempts,
10,000 tool calls, and a seven-day durable deadline (604,800 s). Tokens are
unlimited by default. Each dimension is a positive integer; `"unlimited"` is
refused on a single dimension because a job's durable budget policy is either
every dimension finite or none of them, and `budget_mode = "uncapped"` is the
one spelling that removes every ceiling at once. A config that still states an
old 0.4.2 default keeps exactly the ceiling it asked for.

`wall_clock_seconds` is an optional progress guard: unset means no wall-clock
guard, attended or not, and the no-progress guard is what ends a stuck job. A
configured value is honoured exactly. `max_cost_usd` is optional and stops the job when the provider-reported cost
(OpenRouter's `usage.cost`) passes it; when a provider reports no cost, the
guard cannot fire. `budget_mode = "uncapped"` removes the backstops only — the
progress guards still apply unless their counts are `0`.

The coding root and its helpers advertise `shell` whenever the mode admits one.
`[tools] allow` is the one switch that removes it — naming no `shell` there
takes it from the root and from every helper at once — and under
`--approval-mode never` no surface advertises it at all. A call runs on the host
by default and asks the operator under `ask`, or runs in the sandbox when the
call sets `sandbox: true`; see `docs/commands.md` for the lanes, the conditions,
and the sandbox's rules. A helper that cannot write has a sandbox-only shell
with nothing writable.

Do not set numeric aggregate caps alongside this mode; resolution rejects that
conflict instead of silently ignoring either setting. The mode does not remove
concurrency, child, command, per-response output, context-size, process-timeout,
authority, cancellation, or SQL safety bounds. A project config cannot enable
it unless the operator explicitly trusts project configuration. The ledger
still enforces its 10,000-reservation and 1,024-live-permit limits, the 2-command
slot cap, and the 65,536-event resume scan. This mode has no configured
aggregate budget ceilings, but it is not unlimited storage or work. The
progress guards still apply unless `no_progress_turns` and `repeat_limit` are
both `0`.

Start and resume approval requests show the selected budget mode. The job's
durable policy keeps that selection fixed: a bounded job cannot resume as
uncapped, and an uncapped job cannot resume as bounded. Existing V1 policy
records continue to mean bounded mode. Any independent verified token-budget
policy remains in force in either mode.

In bounded mode, `limits.job.known_tokens` is a per-loop limit on
provider-reported tokens: each model loop stops once its own reported tokens
reach the limit, so one in-flight request can overshoot it.

In bounded mode, the job-wide total is not limited by default.
`limits.job.job_tokens` is the optional job-wide limit across every model loop
of a job, parent and children included; leaving it unset means no
job-wide ceiling. When set, it is checked before each new model request from
provider-reported usage. Concurrent requests already admitted may settle after
the reported total reaches the ceiling and push it further above the limit.
Usage the provider does not report is not counted. A value of `0` is refused as
a typo, and values above `100_000_000` are refused as out of range.

```toml
[limits.job]
no_progress_turns = 8        # 0 disables the guard
repeat_limit = 3             # 0 disables the guard
# wall_clock_seconds = 7200  # unset: no wall-clock guard
# max_cost_usd = 5.0         # optional provider-reported cost ceiling

logical_requests = 2000      # runaway backstops, not spend targets
transport_attempts = 6000
tool_calls = 10000
# known_tokens = 400000      # optional per-loop ceiling; unset means unlimited
# job_tokens = 5000000       # optional job-wide ceiling; unset means unlimited
elapsed_seconds = 604800
```

`config doctor` reports paths and selection. `config show` emits the resolved
configuration as display-safe references and settings only. It never resolves
or prints secret values — that is not optional and there is no flag to change
it.

The REPL session directory uses `SAYA_SESSION_DIR` first, then
`$XDG_DATA_HOME/saya/sessions`, `%APPDATA%/saya/sessions`, or
`~/.local/share/saya/sessions`. In non-interactive mode, an omitted
`--approval-mode` resolves to `never` (schema-only); interactive mode defaults
to `ask`.

Local state uses `SAYA_STATE_DB` when set; otherwise it is stored at
`saya/state.sqlite3` beneath the platform data directory. The SQLite database
contains complete schema snapshots under opaque profile IDs and a bounded,
typed audit log. It never stores credentials, secret references, connection
URLs, SQL, prompts, result rows, provider payloads, headers, driver errors, or
source file paths. Session JSON behavior and `SAYA_SESSION_DIR` are unchanged.

SAYA connects to PostgreSQL, MySQL, SQLite, Snowflake, ClickHouse,
and BigQuery. Environment and file secret
references are resolved at runtime without serializing or logging their values;
keyring references return an explicit unavailable error. Provider settings may
use either the established `SAYA_AI_PROVIDER`, `SAYA_AI_MODEL`, and
`SAYA_AI_BASE_URL` names or the shorter `SAYA_PROVIDER`, `SAYA_MODEL`, and
`SAYA_PROVIDER_BASE_URL` names. `SAYA_API_KEY` becomes a runtime
`{ env = "SAYA_API_KEY" }` reference and is never serialized. Ollama, OpenAI,
OpenAI-compatible, Anthropic, and Gemini providers are supported. Fully offline
agent use is unavailable: the agent still needs a configured provider endpoint.

Schema discovery is automatically allowed. A bounded SQL tool call is allowed
under `read-only`, denied under `never`, and asks for explicit `y/yes` on a TTY
under `ask`; it is denied when a TTY is unavailable. Model responses are streamed
for Ollama, OpenAI, OpenAI-compatible, and Anthropic providers, while Gemini
replies are returned buffered. Session files persist redacted
user/assistant turn text and safe tool name/status metadata, but omit tool
payloads, credentials, headers, and raw tool-result rows. A database-derived
assistant turn may still contain values in its natural-language answer and is
persisted locally after redaction; it is omitted from cloud provider history
when sharing is disabled. OpenAI, OpenAI-compatible, Anthropic, and Gemini
providers are treated
as cloud: with sharing disabled, schema metadata may be sent but the SQL tool
is hidden and dispatcher-blocked, so rows cannot reach those providers. Ollama
is treated as local in this MVP. Interactive `/privacy`, `/model`, `/provider`,
and `/connect` overrides apply to the next prompt; `/include` (and
`--include-profile`) connect additional read-only databases for multi-database
agent navigation.
Interactive prompts and resumed sessions reconstruct only bounded, redacted
user/assistant provider history. `/clear` clears visible, persisted, and
provider context. Raw tool arguments, tool responses, and raw tool-result rows
are never restored into provider history. Local Ollama history may include
database-derived turns; v1 sessions without settings use the current runtime
defaults.

## Instruction files

Project and personal instruction files are read as context data; they grant
nothing and never change permissions or policy. The loader reads, in order:

1. the personal `instructions.md` in the user config directory, when
   `[instructions] personal` is on;
2. the workspace-root `AGENTS.md`;
3. the `AGENTS.md` in each directory between the
   workspace root and the current working directory, top-down and including
   that directory. Nothing else is walked: files elsewhere in the tree are
   never discovered.

Each directory contributes at most one file, and at most 64 nested directories
are read. The project files share one 32 KiB allowance, admitted root first;
the personal file keeps its own 16 KiB allowance, and the combined total is
48 KiB. When a file does not fit, its admitted body is a prefix and the cut is
stated in `/context` and in the context report — truncation is never silent.
The limits are lowerable, never raisable, in `[instructions]`:

```toml
[instructions]
project = true          # workspace-root and nested AGENTS.md
personal = true         # user-config instructions.md
project_max_bytes = 32768   # root plus nested files, shared
personal_max_bytes = 16384
total_max_bytes = 49152
```

Reads are contained in the workspace and never follow a symlink: a symlinked
file or directory component is refused and reported as skipped, so a link
cannot pull an outside file into context. Missing files are silent; a file
that is a directory, invalid UTF-8, unreadable, or too large is reported as
skipped with its reason. `/context` lists every loaded file by path, size,
truncation and redaction counts, and every skipped or budget-omitted source,
without printing its text.

## Removed in 0.5.0

Twenty-nine settings were retired or moved. Each one now fails to parse, and
the diagnostic quotes the message that names the fix:

| Retired setting | What to do |
| --- | --- |
| `[coding] enabled` | Delete the line. Coding is always available. |
| `[coding] check_profiles` | Delete the section. Checks no longer run; the model runs tests itself with `shell`. |
| `[coding] max_concurrent_commands` | Delete the key. Owner commands no longer run. |
| `[coding] max_children` | Move the key to `[agents] max_children`. |
| `[coding] max_concurrent_model_requests` | Move the key to `[agents] max_concurrent_model_requests`. |
| `[coding.agents]` | Move the section to the top-level `[agents]` table. |
| `[coding] budget_mode` | Write `limits.job.budget_mode`. |
| `[coding] max_logical_model_requests` | Write `limits.job.logical_requests`. |
| `[coding] max_physical_transport_attempts` | Write `limits.job.transport_attempts`. |
| `[coding] max_tool_calls` | Write `limits.job.tool_calls`. |
| `[coding] max_known_tokens` | Write `limits.job.known_tokens`. |
| `[coding] max_job_tokens` | Write `limits.job.job_tokens`. |
| `[coding] max_elapsed_seconds` | Write `limits.job.elapsed_seconds`. |
| `[coding] shell` | Delete the key. Remove `shell` from `[tools] allow` instead — that list is the one switch for what a surface advertises. |
| `[coding] program_dir` | Write `job.program_dir` in the `[job]` table. |
| `[coding.extensions]` | Write `[job.extensions]`. |
| `[coding] automatic_lsp_profile` | Write `job.automatic_lsp_profile`. |
| `[ai] investigation_max_logical_answering_requests` | Write `limits.session.requests` (and `limits.ask.requests` for the one-shot surface). |
| `[ai] investigation_max_requested_tool_calls` | Write `limits.session.tool_calls` (and `limits.ask.tool_calls`). |
| `[ai] investigation_max_elapsed_seconds` | Write `limits.session.elapsed_seconds` (and `limits.ask.elapsed_seconds`). |
| `[ai] investigation_max_known_reported_tokens` | Write `limits.session.known_tokens` (and `limits.ask.known_tokens`). |
| `[jobs] wall_clock_seconds` | Write `limits.run.wall_clock_seconds`. |
| `[jobs] tokens_per_endpoint` | Write `limits.run.tokens_per_endpoint`. |
| `[jobs] turns` | Write `limits.run.turns`. |
| `[jobs] tool_calls` | Write `limits.run.tool_calls`. |
| `[jobs.runner]` | Move `program_dir` to `[job] program_dir` and delete the section. `allow` and `timeout_seconds` have no replacement. |
| `[jobs.interpreter]` | Delete the section. |
| `[run] max_iterations` | Delete the key. `limits.run.turns` is the run's turn ceiling, and `SAYA_MAX_ITERATIONS` is no longer read. |
| `[coding]` (whole table) | Rename it to `[job]`; its ceilings live in `[limits.job]`. |

The messages, exactly as the parse error renders them:

```
`[coding] enabled` was removed; coding is always available. Delete the line.
[coding] check_profiles was removed; checks no longer run. Delete the section.
[coding] max_concurrent_commands was removed; owner commands no longer run. Delete the key.
[coding] max_children was moved: helper agent types and the bounds they run under live in the top-level [agents] table. Write agents.max_children there instead.
[coding] max_concurrent_model_requests was moved: helper agent types and the bounds they run under live in the top-level [agents] table. Write agents.max_concurrent_model_requests there instead.
[coding.agents] was moved: helper agent types live in the top-level [agents] table. Move the section to [agents].
[coding] budget_mode was moved: write limits.job.budget_mode instead.
[coding] max_logical_model_requests was moved: write limits.job.logical_requests instead.
[coding] max_physical_transport_attempts was moved: write limits.job.transport_attempts instead.
[coding] max_tool_calls was moved: write limits.job.tool_calls instead.
[coding] max_known_tokens was moved: write limits.job.known_tokens instead.
[coding] max_job_tokens was moved: write limits.job.job_tokens instead.
[coding] max_elapsed_seconds was moved: write limits.job.elapsed_seconds instead.
[coding] shell was removed: the [tools] allow list is the one switch for what a surface advertises. Remove `shell` from [tools] allow instead.
[coding] program_dir was moved: the job host's own settings live in the [job] table. Write job.program_dir there instead.
[coding.extensions] was moved: write [job.extensions] instead.
[coding] automatic_lsp_profile was moved: write job.automatic_lsp_profile instead.
[ai] investigation_max_logical_answering_requests was moved: write limits.session.requests instead.
[ai] investigation_max_requested_tool_calls was moved: write limits.session.tool_calls instead.
[ai] investigation_max_elapsed_seconds was moved: write limits.session.elapsed_seconds instead.
[ai] investigation_max_known_reported_tokens was moved: write limits.session.known_tokens instead.
[jobs] wall_clock_seconds was moved: write limits.run.wall_clock_seconds instead.
[jobs] tokens_per_endpoint was moved: write limits.run.tokens_per_endpoint instead.
[jobs] turns was moved: write limits.run.turns instead.
[jobs] tool_calls was moved: write limits.run.tool_calls instead.
[jobs.runner] was removed; run_program no longer exists. Move program_dir to [job] program_dir and delete the section.
[jobs.interpreter] was removed; run_program no longer exists. Delete the section.
[run] max_iterations was removed; `limits.run.turns` is the run's turn ceiling. Delete the key.
[coding] was renamed: the job host's own settings live in the [job] table, and its ceilings in [limits.job]. Move the section.
```

The tools and flags that went with them — `run_program`, the `--program` flag
on `saya start`, checks and check profiles, and every `coding_*` model tool —
are listed in the [changelog](../CHANGELOG.md).
