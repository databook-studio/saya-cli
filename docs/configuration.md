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

Environment-only DuckDB uses `SAYA_DB_TYPE=duckdb` and `SAYA_DB_PATH`. A
file-backed path must also set `SAYA_DB_READ_ONLY=true` or `false`; `:memory:`
may omit it. `SAYA_DB_READ_ONLY` controls DuckDB file access mode and is
distinct from global `SAYA_READ_ONLY`, which controls the SQL/query policy.
Only the exact strings `true` and `false` are accepted for this setting.

Environment-only SQLite uses `SAYA_DB_TYPE=sqlite` and `SAYA_DB_PATH`, with optional
`SAYA_DB_READ_ONLY` (defaults to `true`). `:memory:` is unsupported. SAYA
enforces bounded read-only SQL regardless of the setting.

The `[run]` table sets execution limits and the query policy:

```toml
[run]
read_only = true            # SAYA_READ_ONLY overrides this
max_rows = 1000
max_iterations = 12
query_timeout_seconds = 60
```

`read_only` (overridable by `SAYA_READ_ONLY`) is the global SQL/query policy and
also drives session-level read-only on connectors that support it. This is
distinct from a profile's own `SAYA_DB_READ_ONLY`, which sets a file engine's
(DuckDB/SQLite) access mode.

`max_iterations` (default `12`) is a stored setting with no behavioural
reader: `[jobs] turns` is opt-in, and unset means unlimited. A zero is
refused as a typo.

The `[jobs]` table sets the default budgets a `saya run` is declared with when
its specification and each of its steps declare none. Each key is optional and
independent; the run pauses when a declared budget trips rather than silently
stopping.

```toml
[jobs]
turns = 40                # per-episode turn ceiling
tool_calls = 25           # per-episode tool-call ceiling
wall_clock_seconds = 1800 # run wall-clock ceiling

[jobs.tokens_per_endpoint]
"local-ollama" = 200_000  # token ceiling per named endpoint
```

`turns` has no default: unset means no ceiling, like `wall_clock_seconds`
and `tool_calls` — a ceiling left unset is unlimited at the contract level,
and the run pauses when a declared budget trips rather than overrunning. `tokens_per_endpoint` is
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

A zero there is refused the same way. No `[jobs]` key has an environment
override, deliberately: a run must be reproducible from its specification and
config alone. Per-invocation overrides belong on the command line instead —
`saya run --budget turns=40 --budget tokens.orchestrator=100000` — whose known
keys are `wall-clock=<seconds>`, `turns=<n>`, `tool-calls=<n>`, and
`tokens.orchestrator=<n>`: today every episode calls the orchestrator
endpoint, so a `tokens.<role>` ceiling naming any other endpoint is refused at
start (the map's shape is still validated here, at config resolve time).
Unset keys fall back to `[jobs]`, and a zero is refused as a typo there too.

`[jobs.runner]` declares the runner universe: the programs a run's
`--allow runner:<name>` may draw from, the one directory they are staged in,
and the default wall-clock ceiling for one child process:

```toml
[jobs.runner]
allow = ["bench"]                    # the programs a runner scope may name
program_dir = "/opt/saya-programs"   # where those programs are staged
timeout_seconds = 300                # per-child ceiling; default 300
```

`allow` entries are bare program names — never paths — bounded at 32, with
no repeats; a shell or interpreter name (`bash`, `python3`, `env`, …) is
refused at resolve time, because the runner runs one allowlisted program
with typed argv and an interpreter would spawn arbitrary children from
inside the allowlist. An `allow` that names programs **requires**
`program_dir`, an absolute path to the operator-owned directory the
programs are staged in; a relative path is a typed resolve error (the
canonical form must not depend on the working directory the config was
loaded from), while `program_dir` alone — an empty `allow` — is harmless.
Existence is deliberately not checked here: a dangling path must not break
`saya ask` or `saya query`, and a run that approved the runner fails closed
at start instead.

The directory is operator-owned and staged by you, before the run: the
engine never writes it, at claim or at any other point. Stage each
allowlisted program as a regular, non-symlink, non-script file with its
bare name — a symlink, a shebang script, or a missing file refuses the run
at start (exit `3`) naming the program and the directory. The directory
must also sit outside the run's filesystem roots in both directions — not
inside, equal to, or containing one — and the run refuses to start
otherwise: with programs inside the run tree, one step's child could write
the binary the next step's `run_program` validates and executes. The same
guard runs for interactive sessions against the session's workspace root —
the project tree — so a project's checked-in tool directory, inside a
session's fs root, is refused for sessions (the enforcement cannot express
an exclusion: Seatbelt subpaths are allow-lists and Landlock has no
subtractive rights); keep a session's program directory outside the
workspace tree, in the default recommended layout beside the runs and
sessions roots. The runner tool itself is admitted only where the startup
sandbox probe proved the host; on a host the probe refused, plans asking for
the runner refuse as needs-approval.

The project layer may set `[jobs]` without `--trust-project-config`: it is a
cost control, not a security-critical setting. Layering is per key: a layer
that declares a key replaces that key's whole value from the lower layers, so
the `tokens_per_endpoint` map and the `[jobs.fetch]` sub-table are replaced
wholesale rather than merged field-wise.

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
guarantees are `run_program`'s, not this one's. Under bypass, a hostile
workspace file is effectively arbitrary code execution as the user.

`[session_commands] deny` states the session's deny list of bare program
names. User-layer only — a project-layer `[session_commands]` is a typed
resolve error — and refusal-only: it composes nothing, and gates every
session door that execs a program by name (`run_command`, `run_program`, the
interpreter door), before every grant, every approval prompt, and bypass, in
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
`--theme <dark|light|auto>` global flag overrides this for a single invocation
and follows the usual CLI-over-config precedence.

The `[ai]` table tunes the provider request the agent loop assembles.
`context_byte_budget` is the ceiling on the approximate byte size of the whole
conversation sent to the provider (system prompt, user question, history, and
the tool results the loop appends). The default is 256 KiB. When the assembled
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

`context_window_tokens` declares the model's context window in tokens. saya
keeps a small built-in table of published models — GLM, GPT, Claude, Gemini,
and the common Ollama families — and looks a model up by exact name, so
`glm-5.2` and `qwen2.5-coder:14b` resolve to their documented windows without
any configuration. A model the table does not know stays **unknown**: most
saya users run through a gateway serving models the table will never list, and
guessing a window for one would either refuse work that would have succeeded or
promise headroom that does not exist. Nothing blocks or truncates on this value
yet; it establishes the fact that a later change can act on.

For a gateway model the table will never hear of, declare the window yourself —
a declared value wins over the table, because it says something about *your*
endpoint that a published fact for the model name cannot:

```toml
[ai]
model = "my-gateway-model"
context_window_tokens = 1048576
```

A declared value of `0` is rejected as a typo. There is no upper bound.

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

SAYA connects to PostgreSQL, MySQL, SQLite, DuckDB, and Snowflake. Environment and file secret
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
