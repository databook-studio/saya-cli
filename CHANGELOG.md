# Changelog

All notable changes to SAYA CLI are recorded here. This project follows
[Semantic Versioning](https://semver.org).

## 0.3.3 — 2026-09-03

### Changed (breaking, for users of the library crates)

- **`ChatRequest`, `ChatResponse` and `TokenUsage` are now `#[non_exhaustive]`,
  and are built through constructors.** These three types gained fields in this
  release — three on `TokenUsage`, two each on the others — and every addition
  broke struct-literal construction in any crate outside `saya-agent`. Marking
  them stops that recurring: fields may be added from now on without breaking a
  downstream build. The enums beside them were already `#[non_exhaustive]`; the
  structs were not, which was an oversight rather than a decision.

  Construct them with `ChatRequest::new(model, messages)`,
  `ChatResponse::new(message)` and `TokenUsage::new(input, output)`, then attach
  the optional parts: `with_tools`, `with_response_format`,
  `with_reasoning_effort` on a request, and `with_cached_input`,
  `with_cache_creation`, `with_reasoning` on usage. The usage builders take
  `Option<u64>` so a call site still says plainly whether a count was reported
  at all — `None` is "the provider did not say", which is not `Some(0)`.
  Reading these types is unchanged; only construction moves.

### Added

- **Show the model's chain-of-thought on demand.** A new `[ai] show_thinking`
  setting (default off), a `--show-thinking` flag, and a `/thinking` slash
  command toggle the display of the model's reasoning in the transcript. It
  arrives once per provider round-trip rather than token by token, so on a turn
  that calls tools it appears in installments, before each call. Off by
  default: thinking is verbose (measured at ~2x the answer length) and restates
  database contents in prose, so a user who did not ask for it never sees it. When on, reasoning renders as a dimmed block visually
  subordinate to the answer — never mistakable for it — using the existing
  secondary style. What is shown is never stored: reasoning lives on the
  per-call `ChatResponse` and the in-memory `ReasoningText` event, neither of
  which has a field on the persisted `SessionLine` or `RedactedTurn`, so a
  session saved while thinking is on contains none of it. `Ctrl+B` (copy
  transcript) and `Ctrl+Y` (copy last answer) exclude thinking blocks — the
  clipboard is a channel off-screen, and model prose that may restate row
  values belongs on screen to the person already reading the answer, not on
  the system clipboard; `/help thinking` names this, and names the one path
  that is not filtered: selection mode (`Ctrl+O`) hands the screen to the
  terminal, whose own drag-select cannot be filtered, so entering it while
  reasoning is visible says so. The `/thinking` toggle
  affects only subsequent turns: reasoning from earlier turns was not retained
  and cannot be re-rendered. `show_thinking` is not security-critical — it
  renders locally to the person who already sees the answer and cannot
  exfiltrate anything the answer does not already show — so the project layer
  may set it without `--trust-project-config`. The headless renderer stays
  silent for reasoning events (a pipe has no transcript), and a content event
  still reaches the loud path so the silence is not a blanket one.

- **`ChatRequest` carries a `reasoning_effort` the extraction call sets to
  `Minimal`, alongside the JSON mode it already set.** This is the honest lever
  for "think less" — ask for it directly rather than suppressing reasoning as a
  side effect of the response shape. The two are separate: `response_format` is
  the answer's form, `reasoning_effort` is how hard to think. The extraction call
  sets both, because the JSON shape is the only mechanism measured to actually
  cut the chain-of-thought on the gateway in use, while the effort hint is the
  correct lever that other endpoints honour; dropping JSON mode would silently
  restore the multi-second waits, so the mechanism that works stays and the
  correct lever is added alongside it. Support for the effort hint is
  endpoint-dependent and frequently a no-op: against the configured gateway
  (`glm-5.2`), `reasoning_effort: "minimal"` produced 223 reasoning tokens
  against a 270-token baseline — the hint was accepted and ignored. saya reports
  what it asked for, never that the effort was applied; whether the model
  complied is only knowable from the reported reasoning tokens. The main agent
  loop is unchanged: it sends nothing, leaving effort to the endpoint, so a
  self-hosted gateway operator's own configuration wins and the main loop keeps
  real reasoning. Each provider translates the variant it honours — OpenAI's
  `reasoning_effort` string, Ollama's `think` boolean, Anthropic's and Gemini's
  thinking token budgets — or drops it; a provider with no equivalent still
  works.

- **`/usage` counts the learning call, labelled apart from the answer.** Every
  turn makes two provider calls: the one that answers, and the extraction call
  that decides what to remember. Only the first was counted, so the session
  total silently omitted a call you paid for — and because extraction is
  invisible in the transcript, nothing else would have revealed the omission. A
  total that quietly leaves something out makes every number beside it suspect.
  The two are reported separately rather than merged, since the point is to see
  what learning costs. A provider that reports no usage for the extraction call
  still adds nothing to the total and stays distinguishable from one that
  reported zeros; a timed-out extraction reports whatever the provider billed
  rather than nothing at all.

- **`contracts approve-all` approves the whole review queue — and reports
  every item it refused.** Candidates were only ever confirmable one at a time
  (`contracts decide --confirm <prefix>`), and a measured store held 26
  pending candidates the user had been told about 26 times without acting —
  the gap was the action, not the notice. The batch approves the same bounded
  queue `contracts queue` shows (default limit 50, `--limit` overrides, clamped
  to 200; the active profile by default, `--profile` to name another) and sends
  **every** candidate through the same per-item validation the single-item
  confirm applies, so a batch is expected to be a mixture: an item dismissed
  between the queue read and the sweep is refused as a conflict, and a
  candidate whose object the cached schema can no longer vet is refused rather
  than rubber-stamped. Nothing is hidden and nothing is rolled back: the queue
  is printed first on every path, then each approval and each refusal is
  reported by claim id with the reason it was refused, and the summary names
  both counts. Without `--yes` the command prints the queue and approves
  nothing (the same deny-by-default `--non-interactive` applies to approvals),
  so a script cannot bulk-confirm by accident; with `--yes` the sweep runs.
  Exits 0 when anything was approved (or nothing was waiting) and 2 when every
  item was refused. A partial batch is a success by design — approving 22 of
  26 and naming the 4 refusals is the correct outcome. Slash surface:
  `/approve-all [--yes] [limit]`. The recall receipt now also points at the
  action the learn path already named: unconfirmed claims render as
  `(N unconfirmed — review with /queue)`, and a recall that found nothing (or
  found only confirmed claims) stays silent as before.

### Added — earlier this cycle

- **`/usage` shows session token totals and a cache hit rate that can say
  "unknown".** The TUI printed one line per turn — `9828 tokens in · 1082
  tokens out` — never accumulated, never showing cache or reasoning spend, and
  nothing for the second provider call (extraction) that also spends tokens. A
  session that ran twenty turns gave you twenty numbers and no total. `/usage`
  now sums every field S21 added (input, output, reasoning, cached input, cache
  creation) across the session and shows the cache hit rate as
  `Σcached / Σinput` — a ratio of sums, not a mean of per-turn rates, stated in
  the command's help text and in the breakdown itself so a reader knows what
  the number is. A field no provider reported renders as `—`; the hit rate
  renders as `unknown` when no turn reported cached tokens, never `0%`. This is
  why S21 made the fields `Option`: a provider that omits `cached_tokens` is
  not reporting a cache miss, and a rate computed over a missing denominator is
  unknown, not zero. The per-turn line stays; `/usage` is the breakdown.

### Added — earlier this cycle

- **The model's chain-of-thought is captured — and cannot be persisted.** A
  reasoning model's thinking was generated, billed, and dropped on the floor:
  `glm-5.2` returns a `reasoning_content` field on every response, including
  "reply with the single word: ok", and saya never read it. It is now parsed
  into a new `ProviderEvent::ReasoningDelta`, accumulated per turn under the
  same `MAX_STREAM_BYTES` bound as content (a hostile endpoint cannot stream
  unbounded "thinking" into memory), and carried on `ChatResponse.reasoning`
  as `Option<String>`. Four providers parse it: OpenAI-compatible
  `delta.reasoning_content` (with `reasoning` as an alias — providers differ on
  the spelling), Anthropic `thinking_delta.thinking` and the `thinking`
  content block, Gemini parts marked `thought: true`, Ollama `message.thinking`.
  A provider that reports no reasoning leaves it `None`, no error, no behaviour
  change. The field lives on `ChatResponse` — transport for one call — and
  **never** on `ChatMessage`, which is what gets replayed to the provider as
  history and what session persistence is shaped around. With no reasoning
  field on `ChatMessage`, "reasoning is never written to a session file" and
  "reasoning is never replayed as history" are structural: there is nothing for
  a session writer or history builder to copy. Capture is unconditional; the
  `show_thinking` toggle that gates *display* is a later slice. This slice ends
  when reasoning reaches `ChatResponse`.

- **The model's chain-of-thought is carried across the crate boundary — and
  still cannot be persisted, and still is not displayed.** The prior slice
  captured reasoning on `ChatResponse.reasoning` inside `saya-agent` and bound
  it to a turn-local it dropped. This slice forwards that dropped value onto
  the event stream as a new `AgentEvent::ReasoningText { text }`, mirroring
  `AssistantText` (one event per turn, the accumulated string), so the CLI
  *can* reach the thinking. It crosses a renderer that has turned three prior
  events into `Not implemented: unrecognized agent event` printed under a
  correct answer in the headless `saya ask` path, each time with a green
  suite (`KnowledgeLearningSkipped`, `KnowledgeProposed`,
  `KnowledgeLearningStarted`): `terminal_event` renders `ReasoningText` to
  `None` — silent, not an error — and a test pins both that and that a content
  event still reaches the loud path, so the fix is not a blanket silence. This
  is the one case where "renders to nothing" is a scope decision (display is
  the next slice) rather than a nature-of-the-event decision (reasoning is
  content, not progress); the comment names that so a future reader does not
  conclude reasoning is progress. The TUI accepts the event and pushes nothing
  to its transcript — display, and the `show_thinking` / `--show-thinking` /
  `/thinking` toggle that gates it, is the next slice. The non-persistence
  guarantee survives the crossing: reasoning lives on `ChatResponse` and on the
  in-memory `ReasoningText` event, never on `ChatMessage`, so a session
  (`SessionLine` / `RedactedTurn`, role + content only) and the replayed
  provider history have nowhere to copy it — a test pins that a session
  persisted after a reasoning turn contains none of it. A provider that reports
  no reasoning emits nothing, byte-identical to today.

- **The extraction call's token spend is no longer invisible.** The

- **The extraction call's token spend is no longer invisible.** The
  non-streaming `complete()` path now carries the usage a provider reports on
  the returned `ChatResponse`, instead of dropping it. Three of four providers
  (OpenAI, Anthropic, Ollama) already route `complete()` through `collect()`,
  which drove `stream()` and threw the `Usage` event away; `collect()` now
  threads the last usage event through. Gemini overrides `complete()` and
  bypasses the stream, so its parsed `usageMetadata` is threaded directly. The
  field is `Option<TokenUsage>`: `None` means the provider reported nothing,
  distinct from `Some(TokenUsage::default())` — a silent provider is not
  mistaken for a free one (absent is not zero, as in the prior slice). The
  `let _ = usage(&body);` discard in the Gemini parser is gone; that parsing now
  earns its keep. Nothing displays this yet — session totals and `/usage` are
  the next slice; this one ends when `complete()` returns the numbers.

- **Token usage can report what it does not know.** `TokenUsage` gains three
  optional fields — `cached_input_tokens`, `cache_creation_input_tokens`, and
  `reasoning_tokens` — parsed from each provider's wire shape (OpenAI
  `prompt_tokens_details.cached_tokens` / `completion_tokens_details.reasoning_tokens`,
  Anthropic `cache_read_input_tokens` / `cache_creation_input_tokens`, Gemini
  `cachedContentTokenCount` / `thoughtsTokenCount`). A provider that omits a
  number leaves it `None`, distinct from a reported `0`: a cache hit rate over
  unknown data is unknown, not 0%. `reasoning_tokens` is documented per field as
  inclusive of `output_tokens` on OpenAI but separate on Gemini, so a later
  display layer does not double-count. The two existing counters keep their type
  and meaning, and `TokenUsage` stays `Copy`. Nothing displays these yet — that
  is the next slice.

### Changed — read this before upgrading

- **The wait after an answer is shorter, because the extractor stops paying for
  reasoning it throws away.** Post-turn extraction — the second provider call
  that decides what to remember — ran on a reasoning model that emitted
  thousands of chain-of-thought tokens before a ~500-token JSON answer, none of
  which saya reads. On `glm-5.2` through the Vivanti gateway the same call that
  took 8s (and could run to 25s) now requests JSON mode and returns in roughly
  a second, with the same proposals. JSON mode is set for the extraction call
  only: a turn that answers in prose still answers in prose. The OpenAI and
  Ollama providers translate the intent; Anthropic and Gemini ignore it (the
  prompt already asks for JSON and the fence-stripper still handles wrapped
  output), so a provider that cannot honour it degrades to today's behaviour
  rather than erroring. The 25s timeout stays — it guards a provider that
  ignores the hint, not a problem this fixes.

## 0.3.2 — 2026-08-31 — first run, and an owl

### Changed — read this before upgrading

- **`saya config init` writes your user config directory, not `.saya/`.** The
  starter `config.toml` and `connections.toml` now land in the trusted user
  layer (`~/.config/saya/`, or `$SAYA_CONFIG_HOME/saya/`), so a first run no
  longer warns that it ignored the templates it just wrote. The project layer
  is untrusted for security-critical settings; writing only there meant the
  tool's own onboarding produced a state its own security model rejected, and
  the next command scolded you for it. Pass `saya config init --project` to
  write the old `.saya/` pair — for team-shared, non-secret settings checked
  into a repository. A command run after `--project` warns until you pass
  `--trust-project-config`; that is the trust boundary doing its job, and
  `saya config doctor` explains how to apply the settings.

### Added

- **`config doctor` advises a next step and exits non-zero when the setup cannot
  work.** It keeps its factual lines and adds actionable advice when something
  is missing — `run saya config init` when nothing is configured, or "set the
  referenced environment variable" when a profile's secret does not resolve. It
  exits `3` (connection/config) when no profile is selected or the selected
  profile's secret is unresolved, and `0` otherwise, so a script can tell a
  broken setup from a working one. Warnings (a missing cloud API key, an ignored
  project override) stay `0`.

- **The three first-run failures name an actionable next command.** An
  unreachable AI provider says to start the provider or run `saya config doctor`
  — not to re-run `init`, since a gateway that is momentarily down is not a
  missing config. An unresolvable secret reference says to set the environment
  variable (or a `.env.saya` file with `--env-file`); `init` cannot supply a
  secret.

- **The post-turn learning wait says what it is waiting for.** A turn is not
  over when the answer finishes: extraction, the second provider call that
  decides what to remember, runs inside the same call the TUI awaits. The status
  bar said "thinking" after visible output, which reads as a hang. It now shows
  `running learning`, and the extraction budget moved from 15s to 25s — measured
  over 12 turns (p50 4.7s, max 13.1s, 2 timeouts), because the old cap was
  protecting you from a wait nobody had explained rather than one that was too
  long.

- **A new mascot: saya is an owl whose pupils are terminal cursors.** It replaces
  the diamond in the README and on the TUI splash. An owl watches everything and
  touches nothing, which is what read-only means. The four states — idle,
  thinking, answering, error — are unchanged in kind: the pupils are the glyphs
  that get substituted.

### Fixed

- **`saya ask` no longer prints "Not implemented: unrecognized agent event"
  after a correct answer.** A progress-only event has no headless rendering, and
  the renderer's catch-all turned that into an error line directly beneath the
  reply. Progress events now render to nothing; events carrying content still
  reach the loud path.

- **The TUI empty state no longer sends first-run users to a file that is not
  created.** It said to add a profile to `.saya/connections.toml`, which stopped
  being where `config init` writes.

- **The untrusted-override warning is one line, not four.** It named all four
  settings on every command, which in a repository that ships a `.saya/config.toml`
  meant a wall of text before every `config show` or `connection list` — and a
  warning you see every time is one you learn to scroll past, which costs exactly
  the case it exists for. It stays on every run and on stderr; `config doctor`
  now carries the detail of which settings were ignored and why.

- **`/help` lists commands in described groups.** It printed 28 commands as bare
  syntax over six dense lines with one description among them, so learning the
  surface meant running `/help <name>` 28 times.

### Documentation

- **The README is 136 lines instead of 355.** Roughly 190 of them restated
  `docs/connections.md` and `docs/configuration.md` — TOML for every connector,
  Snowflake auth modes, sslmode tables — and had already gone stale. It now
  carries only what changes slowly; anything enumerable points at `docs/` or at
  `saya --help`, which is generated from the code and cannot drift.

## 0.3.1 — 2026-08-29 — hardening

A hardening pass over 0.3.0: 30 fixes and 20 features across the read-only
safety layer, the agent loop, the connectors, and the CLI/TUI surface.

**Read Changed before upgrading.** Despite the patch version, this release
contains six breaking changes, and three of them can stop SAYA starting or
connecting on a setup that worked in 0.3.0: a stale config key now refuses to
parse, PostgreSQL requires TLS by default, and the project config layer is no
longer trusted for security-critical settings.

### Repository note

Two internal skill files — `.claude/skills/saya-run/SKILL.md` and
`.claude/skills/saya-smoke/SKILL.md` — shipped in the `v0.1.0` initial public
release and were removed in `0.3.0`. They are absent from every current tree,
but a deleted file stays retrievable from git history, so they can still be
read from a clone. Both describe how to launch the CLI and smoke-test the REPL
locally; neither contains credentials or infrastructure detail. The exposure is
recorded and accepted rather than repaired, since removing it would mean
rewriting published release history. See
[RELEASING.md](RELEASING.md#internal-only-paths-must-not-reach-a-public-ref).

### Changed — read this before upgrading

Six changes alter behaviour you may be relying on. Three of them can stop
SAYA starting or connecting on a setup that worked in 0.3.0.

- **An unknown key in `config.toml` or `connections.toml` is now an error.**
  Previously a typo fell back to the default silently — worst case a typo'd
  `sslmodee` dropped TLS enforcement with no signal. The error names the
  offending key and lists the valid ones.

  This bites on upgrade if your config carries a key that no longer exists.
  In particular `retention_days` was removed (it was parsed, merged and
  surfaced in diagnostics while nothing read it), so a config that sets it
  will not start. Delete the key.

- **PostgreSQL `sslmode` now defaults to `require`, not `prefer`.** `prefer`
  lets an active attacker answer the SSL request with a refusal and collect
  the credentials in plaintext. A server that does not offer TLS will now be
  refused rather than silently downgraded — including a local development
  Postgres. Set `sslmode = "disable"` explicitly for those; see
  [connections.md](docs/connections.md). Note `require` encrypts but does not
  verify the certificate: use `verify-full` where you need that.

- **The project layer is no longer trusted for security-critical settings.**
  A repository's `.saya/config.toml` can no longer set `ai.base_url`,
  `ai.api_key`, `ai.allow_data_sharing` or `run.read_only` — a cloned
  repository is untrusted input, and those four decide where your API key is
  sent, whether rows leave the machine, and whether read-only enforcement
  stays on. SAYA warns when it ignores one. Pass `--trust-project-config` (or
  set `SAYA_TRUST_PROJECT_CONFIG`) to accept them.

- **Enter no longer approves a tool-approval prompt.** The prompt can appear
  while you are typing your next message, so an implicit Enter must never
  allow SQL to run. Press `y` to allow; `n` or Esc to deny.

- **`saya contracts review` is removed; use `saya contracts decide`.** Two
  commands confirmed or rejected a claim and `review` was the weaker one: its
  `--confirm` and `--reject` were independent flags, so `--confirm --reject`
  (or neither) was caught only at runtime, with a "choose exactly one" error,
  and it took a 64-character claim id. `decide` takes a single `--decision`
  flag clap rejects at parse time, and the short `ki-xxxx` prefix `contracts
  list` prints. `decide` scopes to a profile and takes `--profile`, so a claim outside
  the active one is still reachable. The slash commands
  `/confirm` and `/reject` already route to `decide` and are unchanged.

  Before: `saya contracts review ki-… --confirm`
  After:  `saya contracts decide ki-… --decision confirm`

  Neither command was documented, and nothing routed to `review` but the CLI
  itself, so the removal should not affect recorded workflows; if a script used
  `review`, swap the line above.

- **`saya config show` no longer accepts `--resolved` or `--redacted`.** Both
  flags were accepted and ignored since the initial release: `config show`
  always printed the one view it has — the resolved, redacted configuration —
  regardless of either flag. A script passing `--resolved` or `--redacted`
  succeeds today and will now fail with an "unexpected argument" error; drop
  the flag. The printed output is unchanged, because the flags never had an
  effect. `--redacted` is gone in particular because a flag that implies
  redaction is optional is worse than no flag — redaction is not optional, and
  the flag invited someone to look for the off switch.

### Fixed

- **The read-only guard now applies to the whole statement tree.** A denied
  function reached through `FROM` as a table function, through `LATERAL`, or
  schema-qualified (`pg_catalog.pg_read_file`, `main.read_csv`,
  `x.load_file`) was accepted. `FOR UPDATE` and `FOR SHARE` inside a derived
  table took row locks on a connection reported as read-only. Both are
  closed, on every backend.

- **Transcript redaction no longer fails open.** A credential header was only
  recognised at the start of a line, so a pasted `curl -H 'Authorization:
  Bearer …'` kept its token. A private-key block whose closing marker was cut
  off — the normal case, since transcripts are byte-capped — was written out
  in full.

## 0.3.0 — 2026-08-20 — conversational memory

SAYA learns your data vocabulary from ordinary conversation and carries it
between sessions. Everything else in this release is secondary to that.

### Added

- **Memory and data contracts** — SAYA remembers typed facts about your tables (a
  reporting time column, an alias, a grain, a column's role) and uses them when
  building later queries.

  **It learns from conversation, not commands.** Say "we count a rental by
  `return_date`, not `rental_date`, because a rental only counts once it comes
  back" while asking an ordinary question, and SAYA answers *and* records the
  fact — including the reason. A later session, in a new process, recalls it and
  says so. `saya contracts remember` still exists for stating a fact directly,
  and `contracts list` / `show` / `forget` / `queue` inspect and reverse what is
  known.

  **Nothing is remembered unless you turn it on.** `[memory] mode` defaults to
  `off`, so upgrading changes nothing; `assisted` enables recall and post-turn
  learning. A fact you state yourself is recorded as confirmed; anything SAYA
  merely infers is a candidate, inert until a human confirms it in
  `contracts queue`. Repetition never promotes a candidate.

  **What reached the model is always visible.** Every turn that used memory
  prints a `memory supplied` receipt naming each claim, so recall is inspectable
  rather than asserted. When SAYA departs from a confirmed claim it says so in
  the answer and prints `memory overridden`. When a turn's learning fails or
  times out it prints `memory not recorded` — a fact you stated is never dropped
  in silence. `--verbose` reports the learning boundary itself: the gate
  decision, the objects involved, the outcome, and how many facts were kept.

  Claims are typed and bounded, not free text: they cannot hold SQL, credentials,
  file paths or instructions, and the store refuses those shapes rather than
  scrubbing them. Recalled context reaches the model as quoted, delimited data
  marked untrusted — never as instruction — so a claim can never enable a tool or
  authorise a query. Every statement still passes the same read-only safety layer.

  Claims know the shape of the object they describe, so a schema refresh marks a
  claim stale when a column it depends on is removed, renamed, retyped or becomes
  nullable — and marks nothing at all when the database simply could not be
  reached. Two confirmed claims that contradict each other are both shown and
  marked disputed rather than silently resolved. Forgetting a claim erases its
  value *and* its reason from the database file, not merely from the API's view.

  Also adds scoped preferences (`saya preferences`) for timezone, date grain,
  output style and default profile.

- **SQLite** — connect to SQLite database files with `type = "sqlite"` (`path`,
  optional `read_only` defaulting to true). Read-only by default and through the
  bounded SQL safety layer; `:memory:` is not supported.

### Changed

- `[memory]` is configured by a single `mode` (`off` | `assisted`). The earlier
  `recall` and `learning` keys are gone.
- A single-valued claim (a grain, a default time column, a column's role) can now
  be corrected: re-stating it with a different value replaces the old one and
  names what it displaced, instead of reporting a duplicate and silently keeping
  the first value.
- `contracts remember` confirms in words rather than echoing a 64-character id.
  Machine-readable output still carries the id.

### Removed

- `contracts import` / `contracts export`. Sharing contract files as TOML is a
  separate concern from conversational memory and was cut from this release.

### Fixed

- **Charts now plot decimal columns.** `NUMERIC`/`DECIMAL` values (e.g. `SUM`/`AVG`
  and money columns) decode to JSON strings; `/chart` and `render_chart` treated
  them as non-numeric and silently dropped them, producing empty bar/line/area/
  scatter charts. Numeric strings are now recognized and plotted.

## 0.2.0 — 2026-08-09

### Added

- **SQL visibility** — the exact SQL a tool is about to run is now shown in the
  approval prompt (both the TUI dialog and the headless `[y/N]` prompt) and
  echoed into the transcript / headless output, so you approve and audit the
  real query text rather than a generic "read-only SQL query" label. In the TUI
  the executed SQL renders as a labelled, multi-line block (broken before each
  major clause) instead of a collapsed one-liner, and the approval panel now
  sits directly above the input box — with the formatted SQL inside it — rather
  than floating in the middle of the screen.
- **Cross-database queries** — a new `bounded_sql_query_all` agent tool runs one
  bounded, read-only query against every connected database in a single
  approval and returns per-database results. Each database runs independently,
  so a dialect mismatch on one is reported alongside the others' successes
  instead of aborting the whole call.
- **Copy & paste from the TUI** — `Ctrl+O` toggles selection mode (releases the
  mouse so your terminal's own drag-select and copy work, with a `SELECT`
  indicator in the status bar); `Ctrl+Y` copies the last answer and `Ctrl+B`
  copies the whole transcript. Copies go to the OS clipboard via the platform
  tool (`pbcopy` / `wl-copy` / `xclip` / `clip`) and also emit OSC 52 so copies
  reach the local clipboard over SSH.
- **Resumed sessions show their history** — resuming a session (via the
  `/sessions` picker or `--resume` / `--continue`) now replays the prior turns
  into the transcript — each question, the tools it ran, and the answer.
- **Launch splash** — before the first question the TUI shows a centered splash
  (name, tagline, your configured databases, example prompts, and key hints)
  instead of an empty panel; it disappears the moment you ask something.
- **Role rail in the transcript** — every transcript line carries a colour-coded
  left rail (you / saya / tool / system / error) so turns read as distinct groups.
- **Colour-coded status bar** — profile in the accent colour, provider/model
  dimmed, approval mode coloured by risk (green read-only, amber ask, red never),
  and privacy coloured.
- **Markdown in answers** — assistant answers render `**bold**`, inline
  `` `code` ``, `#` headings, `-`/`*` bullets, and GitHub-style tables (drawn as
  box tables) instead of raw text.
- **`/export <path>`** — write the last query's results (from `/sql` or an agent
  tool) to a `.csv` or `.json` file. CSV uses RFC-4180 escaping; JSON is an array
  of column-keyed objects.
- **Follow-up refinement** — the agent receives the SQL it most recently ran, so
  a terse follow-up ("now show the lowest instead", "filter to 2023") adapts the
  previous query instead of rediscovering the schema.
- **Charts to interactive files** — `/chart [type]` and a new `render_chart`
  agent tool render the query result as a self-contained, interactive **Chart.js**
  HTML file (bar/line/area/pie/doughnut/scatter) and open it in the browser; the
  AI chooses the chart type when you ask it to visualize data.
- **`/explain [sql]`** — show the `EXPLAIN` query plan for the given SQL, or the
  last query if omitted (full, un-truncated plan text). Read-only; works across
  PostgreSQL, MySQL, DuckDB, and Snowflake.

### Changed

- Numeric columns in result tables are right-aligned so figures line up on their
  digits, while text stays left-aligned.

### Fixed

- **Postgres enum / unknown-type columns** no longer fail a query with an opaque
  "PostgreSQL query failed". User-defined types (e.g. `mpaa_rating`) are decoded
  from their raw text instead of erroring the whole result.

## 0.1.2 — 2026-08-05

### Distribution

- Added an **Intel macOS** (`x86_64-apple-darwin`) build to the release matrix,
  so releases now cover Linux x86_64, macOS arm64, macOS x86_64, and Windows
  x86_64.
- Prepared crates.io publishing: the workspace libraries are now publishable
  (`saya-types`, `saya-config`, `saya-store`, `saya-agent`, `saya-connectors`),
  enabling `cargo install saya-cli`.
- Added `cargo-binstall` metadata so `cargo binstall saya-cli` downloads the
  prebuilt binary instead of compiling.

## 0.1.1 — 2026-08-05

First release published with prebuilt binaries (Linux, macOS, Windows) and
SHA-256 checksums. No user-facing behavior changes versus 0.1.0.

### Dependencies

- Updated `ratatui` 0.29 → 0.30, `toml` 0.8 → 1.1, and `base64` 0.22 → 0.23.

### Build & CI

- Parallelized compilation (removed a one-job throttle) — roughly halved CI and
  release build times for the bundled DuckDB C++ compile.
- Added dependency/build caching (`rust-cache`) and de-duplicated CI runs.
- Release job now builds only (tests and clippy already run on `main`),
  compiling DuckDB once instead of three times.
- Bumped `actions/upload-artifact` and `actions/download-artifact` to current
  major versions.

### Security

- Documented triage of two unfixable/unreachable advisories (`rsa`
  RUSTSEC-2023-0071, `rkyv` RUSTSEC-2026-0235) in `.cargo/audit.toml`.

## 0.1.0 — 2026-08-05

### Interactive full-screen TUI

- Replaced the inline reedline REPL with a full-screen **ratatui TUI**: a
  scrolling transcript, a status bar, and a bordered multi-line input box pinned
  to the bottom.
- Slash-command popup that opens automatically on `/` with **fuzzy** matching;
  Tab/Enter accept, arrow keys navigate, Esc dismisses.
- `@table` / `@table.column` autocomplete from the cached schema of the active
  and included profiles.
- Live streaming answers into the transcript with a spinner, elapsed timer, and
  the currently-running tool; **Esc** cancels an in-flight request.
- Raw SQL in-session via `/sql`, rendered as an aligned table; interactive
  `/sessions` picker (profile / model / turns / age) and resume.
- Tool-approval modal for `approval:ask`; mouse-wheel and PageUp/PageDown
  scrolling; persistent input history; two-stage Ctrl+C; F1 help overlay;
  bracketed paste; input syntax highlighting.
- Non-TTY input (pipes/CI) runs a headless executor; `reedline` and
  `nu-ansi-term` dependencies removed.

### Performance

- Cap rows fed to the model from a query tool at 50 (display path unchanged),
  return a compact schema from `schema_discovery`, and send `temperature` +
  a stable `prompt_cache_key` on OpenAI-compatible requests.
- `[ai].temperature` is now configurable (default `0.1`).

### Earlier

- Added multi-database agent navigation: connect additional read-only databases
  alongside the primary with `--include-profile` (and interactive `/include`),
  and the AI agent inspects and queries any connected database by passing an
  optional `connection` argument to its tools. The agent is told the name and
  SQL dialect of every connected database; a failed secondary connection is
  skipped while the primary run continues.
- Added a native Anthropic (Claude) provider (`provider = "anthropic"`):
  streaming `content_block` parsing, `input_schema` tool declarations, top-level
  `system`, and `x-api-key`/`anthropic-version` headers.
- Added a Google Gemini provider (`provider = "gemini"`): buffered
  `generateContent` with `functionDeclarations`, `systemInstruction`, and the
  `x-goog-api-key` header. All five documented providers (Ollama,
  OpenAI-compatible, OpenAI, Anthropic, Gemini) are now implemented, so
  configuration and runtime agree.
- Added a rich interactive line editor (reedline) with in-session command
  history recall and line editing, plus a status header (active profile,
  included databases, provider/model, approval mode, and privacy state). Piped
  input keeps the plain line reader for predictable scripting/CI.
- Cloud row-data sharing for Anthropic and Gemini is gated on
  `--allow-data-sharing`, consistent with the other cloud providers.
- Added `saya config init` for credential-free project templates.
- Added local archive packaging with checksum and extracted-binary smoke tests.
- Added a manually triggered release-candidate workflow for native CI builds.
- Documented the supported provider, installation, configuration, connection, and
  release boundaries.

## 0.1.0

- Initial private-alpha CLI surface for PostgreSQL, MySQL, DuckDB, Snowflake,
  Ollama, and OpenAI-compatible providers.
