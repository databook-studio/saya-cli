# DESIGN — `run_command`: the unsandboxed second lane

Status: **decision document. No `.rs` file was changed to produce it; nothing
here runs until its slice lands.** Every claim about current behaviour carries
`file:line`, read from the code, not from the task. The `saya-internal`
reasoning the task cites is not a file by that name: the wildcard-grant
rejection lives at `crates/saya-cli/src/grant_token_tests.rs:244-250` and
`UNIFY.md:143-148`, and is cited as such throughout.

## 0. The decision, in one place

1. **Two lanes, two names, two prompts.** `run_program` stays exactly as it
   is — a contained job runner whose prompt states its containment as
   measured fact (`approval_facts/run_program.rs:93-95`). A second tool,
   `run_command`, runs programs **unsandboxed**: the user's uid, whole
   filesystem, full network, resolved on the user's PATH. Its prompt claims
   no containment and says so in the slot where `run_program`'s sandbox line
   sits. The hypothesis is accepted, with refinements argued in §1–§3.
2. **The grant is the program name, and nothing else.** `command:<program>`
   for the session — any argv, unconfined. No prefix grants (`command:cargo
   test`) and no blanket grant (`command:*`): both are fiction, argued in §4.
3. **Every host call is journalled before the child spawns** — a new
   `session-command` event — because this lane is the one place where the
   session's "consent seconds before action" ordering (`UNIFY.md:524-528`)
   is worth its cost.
4. **Bypass runs the lane unasked**, with the activation line stating it; the
   structural refusals (lane not composed, path-shaped names, names not on
   the passed PATH, timeout bounds) still refuse — the same line
   `session_activation.rs:19-20` already draws.
5. **Headless runs never get this lane.** `command:` refuses at parse on the
   run surface, by design, with its own pinned wording (§7–§8).
6. **The lane is off unless stated at launch**: `--host-commands`, or
   `--allow command:<x>` (the seed implies composition), or user-layer
   `[host_commands] enable`. A project-layer `[host_commands]` is a typed
   resolve error — the model can write the project's config once
   `workspace_write` is granted, and a model-writable file must not enable
   unsandboxed execution.

## 1. Today, verified

The session surface: one tool universe shared with runs
(`interactive/session_universe.rs:1-19, 215-258`), a per-call engine whose
tri-choice prompt offers allow-once / allow-session-in-grammar-words / deny
(`protocol/contracts/session_policy.rs:187-204`, `prompt_approval.rs:94-124`,
`grant_token.rs:79-90, 173-177`), grants that die with the process
(`session_policy.rs:54-56`), a journal that records grants and bypass
activations and is never a grant source (`saya-store/src/session_journal.rs:1-16,
45-53`), and bypass as consent-transformation with every structural guard
intact (`session_policy.rs:189`, `session_activation.rs:19-20`).

`run_program`'s four load-bearing properties, each confirmed in code:
bare-name allowlist from `[jobs.runner] allow`
(`saya-config/src/jobs.rs:243-299`, `runner/refuse.rs:52-57, 65-69`;
`is_bare_name` at `saya-types/src/run/scope.rs:40-46`); operator-staged
`program_dir` with the placement guard refusing any containment relation to
the roots (`commands/run/runner.rs:143-192`, `session_runner.rs:185-209`, the
measured escape `docs/run-program-escape-report.md:89-100`); kernel exec
confinement to the program directory only — `(deny default)` plus
`(allow process-exec (subpath <program_dir>))` on macOS
(`runner/sandbox/macos.rs:50, 54-56`), `EXEC_ALLOWED` for program and loader
dirs only on Linux (`runner/sandbox/linux.rs:66-71, 117-133`); and empty
egress in a session (`session_runner.rs:116` — `RunSandbox::new([workspace],
Vec::new())`). Typed argv: no shell, no interpolation, ever
(`runner/mod.rs:1-2, 275-316`, `runner/spawn.rs:55-58`). Since U5 the
approval prompt prints the containment as fact — "sandbox: reads/writes/exec
bounded to <root> · egress: none · cwd pinned to <root>"
(`approval_facts/run_program.rs:93-95`) — under the governing rule "a prompt
may not claim a bound the code does not apply" (`approval_facts/mod.rs:5-13`).

**Why the maintainer's goal cannot ride this lane — four structural bars,
not preferences.** `npm` and `pip` are scripts: the shebang refusal
(`refuse.rs:102-114`) and symlink refusal (`refuse.rs:89-95`) bar them as
staged files. Their interpreters (`node`, `python3`) are refused by name
(`scope.rs:111-137`, mirrored at config resolve `jobs.rs:270-277`). Even a
staged compiled wrapper would face empty egress (`session_runner.rs:116`) —
no registry — and denied `process-fork` (`sandbox/mod.rs:87-93`; the measured
`fork: Operation not permitted`, `session_activation.rs:30-34`), which kills
any real build (`cargo test` shells out to compilers). The lane is a
single-process, no-network, contained lane; package management and builds
are multi-process networked work. Widening it to admit them means deleting
every property that makes its prompt truthful — which is why §0 rejects
widening rather than merely disliking it.

## 2. The lane (Q1)

**Tool: `run_command`.** One program per call, typed argv (the same
contract as the runner: every argument one argv element, no command-line
string anywhere — `runner/mod.rs:275-316`'s dispatch shape, reused). Kept
even though nothing else is kept: typed argv is free, and it kills the
shell-metacharacter class (`;`, `$()`, backticks) at the root — measured
(`docs/run-program-escape-report.md:132-134`).

**"Unsandboxed", precisely:**

- **uid**: the user's own. No privilege change, no namespace, no Seatbelt
  profile, no Landlock ruleset, no probe. Nothing claims confinement, so
  nothing needs proving.
- **filesystem**: the whole filesystem, read, write, and exec, everywhere
  the uid can. Including outside the workspace, including `~/.ssh`, including
  `sessions/<id>/` and `runs/<id>/` (§6 states what that costs).
- **network**: unrestricted, in-process and in-child.
- **program resolution**: a **bare name** (`is_bare_name`, `scope.rs:40-46`)
  resolved against the PATH *value passed to the child*, first regular-file
  match in order; no `program_dir`, no staging, no admission battery — the
  battery exists to keep exec inside one directory (`macos.rs:54-56`), which
  does not apply, so a PATH-resolved symlink or script is *honoured, not
  refused*: the lane's contract is "your toolchain", not "known bytes".
  Path-shaped and traversal names refuse at every layer (the same rule
  `refuse.rs:52-57` applies); a name not on the passed PATH is a typed error
  naming the PATH searched — an honest failure, never a guessed exec.
- **environment**: **built, not inherited** — `env_clear()` then `PATH`,
  `HOME`, `TMPDIR` (the parent's *values* for those names), plus a
  config-declared `pass_env` list. Nothing else reaches the child. This is
  the runner's own discipline (`spawn.rs:59-63`: "a planted variable in the
  parent's environment cannot reach the child"; measured,
  `docs/run-program-escape-report.md:140-142`) kept for the one channel
  where the parent's secrets (the shell's API tokens, resolved DB
  credentials) would otherwise flow into model-chosen execution. The known
  cost: toolchains that want proxy or cert variables need `pass_env`
  entries; the failure is a visible refusal, not a silent gap. Inheriting
  wholesale was considered and rejected (§9).
- **cwd**: pinned to the workspace root — a usability fact (paths in output
  are predictable), stated as cwd and never as a bound. Under bypass the
  standing posture is the same sentence the prompt carries.
- **time and output**: a configured ceiling (`[host_commands]
  timeout_seconds`, default 600 — provisional like `RUNNER_TIMEOUT_SECONDS`,
  `jobs.rs:50-54`), narrowed never widened per call (`refuse.rs:70-82`
  shape); process-group kill and the post-exit orphan sweep
  (`runner/spawn.rs:72-141, 173-194`) shared with the runner by **extracting
  that core into one module** so the two lanes cannot drift on group-kill;
  output through the same ring, cap, and unconditional redaction
  (`runner/output.rs`, `OUTPUT_CAP_BYTES` at `runner/mod.rs:30`).

**How a user turns it on.** Three statements, all human-typed, all before or
outside model influence: the launch flag `--host-commands`; a launch
`--allow command:<x>` (the seed both composes the lane and records the
token — a stated scope is a capability statement, the run surface's own
rule, `scopes.rs:109-121`); or user-layer config `[host_commands] enable =
true` with `pass_env` and `timeout_seconds`. A project-layer
`[host_commands]` is a **typed resolve error naming the layer** — the
existing trust boundary reverts protected keys
(`saya-config/src/resolve.rs:138-146`, `layers.rs:216-242`, with
`jobs_interpreter` already protected at `layers.rs:222`), and this section
goes further: `--trust-project-config` does not unlock it, because the
project file is model-writable (`workspace_write` reaches it once granted).
The lane composes **once per session process**, and only when a workspace
root binds (no-workspace sessions are the database-question shape; the
cwd needs a root) — the composition rule the probe refusal already
establishes: no grant, no bypass, nothing conjures a capability the
composition could not construct (`session_runner.rs:122-130`,
`session_universe.rs:107-113`). Advertisement follows the mode rule
exactly (`session_universe.rs:223-242`): `ask` where a prompt exists,
`bypass` always, `read-only`/`never` hidden-not-advertised. `saya ask`
refuses the flag (typed: the lane is interactive-session-shaped); Windows
fails closed in v1 (the unix group-kill semantics are unverified there —
same posture as the runner's, `sandbox/mod.rs:29-30`, different reason).
The model-facing description states the posture and the preference order:
prefer `run_program` when the program is allowlisted — contained is
strictly safer, and the description can say so honestly.

## 3. The prompt (Q2)

The rule is unchanged: state the bounds the code applies, and nothing else
(`approval_facts/mod.rs:5-13`). What replaces containment facts is **the
absence, said in the same slot** — not silence:

```
run_command — npm
  program: npm — host command: resolved on your PATH, unsandboxed
  argv: npm install
  typed argv: no shell, no interpolation, one element per argument
  no sandbox: runs as your user — your whole filesystem, your network, unconfined
  cwd: pinned to <workspace root>
  environment: PATH, HOME, TMPDIR (built for the child; nothing else reaches it)
  timeout: 600s — the call may narrow it, never widen it
  output: capped at 65536 bytes per stream · redacted before it reaches the model
  what the program spawns, downloads, or executes is not bounded by anything above
  session: no prior host-command grant
  [a] allow once   [s] allow command:npm for this session   [d] deny
```

Rules, stated once: (1) the **no-sandbox line occupies the sandbox line's
coordinates** (`approval_facts/run_program.rs:93-95`'s slot) so the two
postures are visibly different at the same place — a user who has learned to
skim `run_program` prompts meets a different sentence shape here; (2) the
**not-bounded line** is mandatory — the honest inverse of containment facts,
so the prompt can never read as reassurance; (3) every stated fact is the
enforcement's own number (env from the composed builder, timeout from the
resolved ceiling, cap from `OUTPUT_CAP_BYTES`); (4) a shell- or
interpreter-family name (`is_refused_runner_program`, `scope.rs:111-137`)
adds the interpreter warning with the lane's own clause — "the model writes
the program the interpreter runs, and there is no sandbox — it runs as you.
What the interpreter computes is not a reviewed, fixed binary." — the same
no-euphemism builder the two doors already share (`approval_text.rs:15-23`)
with the fork fact replaced by the unsandboxed fact; (5) argv display keeps
the collapse-whitespace defense (`approval_facts/run_program.rs:13-15`) —
the *journal* carries verbatim argv (§4), the *prompt* carries the
unsplitable rendering; (6) once any host command has run in the session,
`run_program`'s prompt gains one fact line — "a host command ran in this
session; staged program integrity is outside saya's control" — because a
prior host child can have rewritten a staged binary (§6), and the sandbox
line alone would then understate. The TUI modal rides the same body — both
frontends render one builder (`approval_facts/mod.rs:1-4`,
`prompt_approval.rs:91-93`), so it cannot drift.

## 4. What a session grant means (Q3)

Three candidate designs were weighed. This is the load-bearing section.

**A blanket grant (`command:*`) is rejected.** It is the wildcard this repo
already refused, in its strongest form: "any 'all' token's referent can
grow after approval" (`grant_token_tests.rs:244-250`) — and here the
referent is not "databases added by `/connect`" but "every program the model
will ever name", unbounded by construction. `--allow all` was refused for
runs because "all is the one token that binds nothing exactly"
(`UNIFY.md:143-148`); a session blanket is worse, because a grant is
*model-initiated* — the model chooses the moment of the ask, and the
prompt is social-engineering surface (`UNIFY.md:528-535`). A blanket
session grant is bypass-for-one-lane wearing a grant's clothes: pressed
once, at a moment the model chose, with none of bypass's ceremony
(indicator, activation line, journal event — `session_activation.rs:44-62`,
`session_journal.rs:89-91`). Bypass exists and is loud; users who want
"everything runs" should reach it, not a keypress that reads narrower than
it is.

**A prefix grant (`command:cargo test`) is a comforting fiction, and is
rejected.** The two prefixes the task names are precisely the two that
refute themselves: `cargo test` compiles and runs `build.rs` — arbitrary
Rust code, by design, at build time; `npm install` runs post-install
scripts — arbitrary code fetched from the network. The prefix bounds the
*string the model typed*, not the *behaviour approved*; the gap between
them is the entire risk. And argv-level matching is defeatable by the
program's own flag grammar (`git -c`, hooks, `--exec-path`, subcommand
re-routing) — the same argument that rejected opencode's `git *` prefix
patterns: "prefix matching over an adversarial space… silent scope creep
inside a granted pattern" (`UNIFY.md:665-672`). The repo already refuses
tokens whose words would lie (`scopes.rs:203-229` — "a token that approved
it here would be a lying scope"); a prefix token lies about its referent at
parse time. Refused, with a typed error that names the fiction.

**The name-only grant (`command:<program>`) is adopted — stated with
exactly what it bounds.** It bounds the model's *direct ask*: which program
is exec'd as the child, resolved on the passed PATH, with any argv. It
bounds nothing about what the program then does — spawns, downloads,
executes, writes — and the prompt's not-bounded line says so. This is the
same honesty shape as the interpreter door's warning ("the model writes the
program the interpreter runs", `approval_text.rs:15-23`): the grant names
what is consented; the warning names what is not. Consequences, stated:
same name, different argv — never re-asked (the grant covers argv-any;
that is the point of a session grant, `UNIFY.md:648-651`); different name —
a fresh ask; each name accumulates for the session's life
(`session_policy.rs:54-56`), and a session that grants twelve programs
holds a wide set — prompt fatigue is the standing cost of per-call approval
(`UNIFY.md:509-518`) and is not fixed here. Suggester: `grant_token.rs`
gains the arm — bare name → `command:<program>`, **no family mirror**
(the `runner:`/`interpreter:` mirror at `grant_token.rs:135-146` belongs to
the sandboxed lane's contract; here `command:bash` is the token, and the
warning rides the prompt); `grant_family` gains the `command` family for
the session-history line (`grant_token.rs:154-168`).

**The journal gains one event kind, per call.** `session-command { program,
argv }`, redacted through the store's existing seam (`session_journal.rs:76-85`
— never a second rule set), written **before the child spawns** —
restoring, for the one lane where it matters most, the consent-before-action
ordering the run journal has and per-call surfaces lose (`UNIFY.md:524-528`).
A crash between journal and spawn over-records, which is the safe direction
for audit. Grants and bypass activations journal exactly as today
(`session_journal.rs:45-53`).

## 5. Bypass (Q4)

Bypass is consent, not containment: it auto-allows every ask and touches no
structural guard (`session_policy.rs:189`, `session_activation.rs:19-20`).
So `run_command` runs unasked under bypass, exactly like every write-shaped
tool — this is the affordance the maintainer settled, and the precedent is
the staged interpreter door, which the bypass activation line already
carries with its warning (`session_activation.rs:44-62`). Two additions make
it honest for this lane: the activation line gains the lane's fact when
composed — "host commands run unsandboxed: as your user, your network, your
filesystem" — and the per-call `session-command` journal event still fires
under bypass (the audit the lane cannot skip). **What still refuses under
bypass**: the lane when not composed at launch (flag, seed, or user-layer
config — nothing conjures it, `session_runner.rs:122-130`); no workspace
bound; path-shaped and traversal names; names not on the passed PATH;
timeout 0 or over the ceiling; Windows. These are composition and
validation refusals, the class that holds in every mode today
(`UNIFY.md:477-480`). Runs keep refusing bypass outright
(`commands/run/start.rs:49-52`, `docs/commands.md:106-107`).

## 6. Prompt injection (Q5)

**Today's worst case**, verified: a hostile file in the workspace is read
unprompted (the unconditional read surface, `SESSION-WORKSPACE.md:168-176`)
and can at most drive a staged, sandbox-contained binary — no network
(`session_runner.rs:116`), fs bounded to the project, no fork, `.git` and
symlinks refused to tools, output redacted, credential channels pinned shut
by the isolation battery (`docs/run-program-escape-report.md:132-154`).

**The new exposure, plainly.** With the lane on, the same hostile file can
drive `npm install` in a repo whose `package.json` it authored — post-install
runs arbitrary code as the user's uid, with full network and the whole
filesystem: exfiltration of the workspace, of `pass_env` variables, and of
anything the uid can read; **persistence** — shell rc files, PATH shims,
git config, and *the staged program_dir itself*, so a corrupted `bench`
validates green in a future session's `run_program` (the S3 escape
`docs/run-program-escape-report.md:89-100`, reachable now through the lane
that was supposed to be outside it — hence the `run_program` prompt line of
§3(6)); and **journal softness** — the session journal, scratch, and even
other runs' journals are uid-writable paths, so a consented child can edit
the audit record. The journal was never evidence against the uid — that is
S3's accepted residual, "the operator's own account" (`SESSION-WORKSPACE.md:
416-418`) — but under this lane the residual is live, not theoretical.

**What bounds it:** only the consent architecture. Per-program asks in ask
mode (a hostile file must get the user to approve `npm`, with the no-sandbox
line in front); the built environment (the shell's secrets stay out unless
`pass_env` names them); typed argv (the direct argv is fully visible in
prompt and journal — no laundering); the off-by-default, launch-stated,
project-scoped opt-in (§2); the per-call journal. **Under bypass, none of
the asks remain: a hostile workspace file is effectively arbitrary code
execution as the user.** That is the honest sentence for the docs; the
design's job is to place it at the moment of choice — launch flag, prompt
lines, bypass activation line — not to shrink it. The lane does not bound
this; it makes it visible.

## 7. The existing runner, and headless (Q6)

**The job runner survives unchanged**, for three reasons: it is the only
lane runs get; it is the honest lane for contained work even in sessions
(`bench`, `ripgrep` — and the ask is cheap because containment holds,
`UNIFY.md:673-679`); and the interpreter door remains the *contained* place
for scripts (staged bytes, empty egress, typed token — `refuse.rs:144-215`,
`grant_token.rs:140-145`), strictly safer than host `python3`. The two
lanes coexist because they are two names with two prompt postures; the
descriptions steer the model to the contained lane when a program is
allowlisted (§2).

**A headless run never gets this lane.** A run is unattended execution over
pre-declared scopes, and its security story leans on structural bounds —
the maintainer's own residual (`docs/commands.md:125-128`). `--allow
command:npm` in CI is a standing blanket approval whose real referent is
"whatever the npm registry and its scripts do, forever", reviewed once at
plan approval where the plan view cannot show what the program does — the
prefix fiction at run scale, plus the rubber-stamp residual with no
containment behind it. Runs also refuse bypass (`start.rs:49-52`), so there
is no consent surface that could honesten it. The existing seed filter
already names-and-drops any `command:` token a session tries to forward
into a `/run` child (`session_grants.rs:159-169`), because the child's
parser refuses them (§8) — the "never silently dropped" rule
(`session_grants.rs:171-175`) carries the explanation.

## 8. The grammar (Q7)

- **New token: `command:<program>`** — payload a bare name
  (`is_bare_name`), shells included (the family mirror deliberately does
  not apply, §4). `KNOWN` gains it (`scopes.rs:29-31`).
- **Session surface**: parses only when the lane composed; with the lane
  off, `/allow command:<x>` refuses with the launch wording — "gates
  nothing in this session; relaunch with `--host-commands`" — the
  parse-time-refusal discipline (`scopes.rs:50-55`) with the composition
  fact threaded to the parser (placement is implementer's latitude; the
  test pins the wording, not the seam).
- **Run surface**: a **permanent policy refusal in its own class** — not a
  `NOT_YET_WIRED` entry, whose discipline is wiring-shaped and whose
  entries leave when their consumer lands (`scopes.rs:44-63`); this one
  never leaves. Its own list with its own pinned reason: "not available on
  runs, by design: a run is unattended and this scope names unconfined
  host execution; state a sandboxed scope instead (`runner:`,
  `interpreter:`). Re-run without it." — never an absence claim
  (`scopes.rs:322-351` discipline).
- **Journal words**: grants as `command:<x>` (`session-granted`, unchanged
  shape, `session_journal.rs:45-53`); the new per-call `session-command`
  event (§4); bypass activations unchanged.
- **Headless runs accept none of it** — `--allow command:x` is an exit-2
  usage error before anything exists on disk (`scopes.rs:1-8`'s
  lying-scope rule, applied at parse).

## 9. What genuinely gets worse; what I refuse to build (Q8)

Worse, stated: (1) bypass + lane = hostile-workspace arbitrary code as the
user (§6) — the maintainer's goal, honestly priced; (2) two prompt
postures to learn, and a model choosing lanes (mitigated by descriptions
and by the slot rule, §3); (3) ask-mode program-name fatigue, wider grant
accumulation than scopes-once would have produced (`UNIFY.md:509-518`
transfers); (4) the audit record is uid-soft (§6); (5) the sandboxed lane's
staged-binary premise degrades once a host command has run (§3(6)).

Refused, though asked or adjacent: **widening `run_program`** — the prompt
would become the reassuring falsehood U5 existed to kill
(`approval_facts/mod.rs:5-13`); **`command:*`** (§4); **prefix grants**
(§4); **the lane on runs** (§7); **`[host_commands]` from the project
layer**, even trust-gated (§2); **env inheritance** — the honest widening
if friction demands it is more `pass_env` names, never blanket inheritance;
**mid-session lane enabling** — a recomposition the model could solicit;
relaunch + `--resume` is the honest path (grants die with the process
anyway, `session_policy.rs:54-56`). Also named and deferred: a doom-loop
repetition guard (`UNIFY.md:656-659`) is worth borrowing for all tools, not
this lane alone.

## 10. Slice order

**H0 — the host executor (saya-harness; no wiring).** New
`crates/saya-harness/src/host/{mod.rs,resolve.rs,env.rs}`; extract the
wait/kill/sweep core from `runner/spawn.rs:86-141, 173-194` into one shared
module (both lanes consume it — group-kill must not drift); reuse
`runner::output` wholesale. Tests, named: `typed_argv_arrives_byte_exact`;
`a_planted_parent_env_var_never_reaches_the_child`;
`timeout_kills_the_process_group_including_a_daemonizing_grandchild`;
`resolution_uses_the_built_path_not_the_parent_s`;
`path_shaped_and_traversal_names_refuse_at_every_layer`;
`a_name_not_on_the_built_path_refuses_with_the_path_it_searched`.
Gate: the harness suite plus these sentinels; no session surface exists
yet, so no integration test can lie.

**H1 — grammar, config, composition.** `scopes.rs` (`command:` family,
surface rules, `KNOWN`, the new run-surface refusal class);
`saya-config` (`[host_commands]`: `enable`/`pass_env`/`timeout_seconds`;
the project-layer typed error, joining the protected-set mechanism at
`resolve.rs:138-146` as a hard refusal); `session_universe.rs` + new
`session_host.rs` (compose once: workspace required, executor + facts);
`session_definitions.rs::run_command` (the honest description +
prefer-contained line); `run_tools.rs` dispatch arm (beside the
`run_program` arm, `agent/tools/run_tools.rs:72`); CLI flag + `--allow
command:x` implying composition; `saya ask` refusal. Tests:
`command_tokens_parse_on_a_composed_session_and_refuse_on_a_bare_one`;
`a_run_refuses_command_scopes_by_design` (pinned wording);
`the_project_layer_cannot_enable_host_commands`;
`a_launch_allowing_command_x_composes_the_lane_and_seeds_the_token`;
`no_workspace_no_lane_even_with_the_flag` (hidden-not-advertised rule,
`session_universe.rs:237-243`);
`ask_refuses_the_host_commands_flag`;
`run_seed_names_command_tokens_as_dropped`;
`the_lane_s_advertisement_follows_the_mode_rule`.
Gate: a session with the lane can run one PATH program via one Ask;
read-only sessions never see it; a bare session's `/allow command:x`
refusal bytes pinned.

**H2 — facts, grants, journal.** `grant_token.rs` (`command_token`, no
mirror; `grant_family` arm); new `approval_facts/run_command.rs` (§3's
body — both frontends ride it, `approval_facts/mod.rs:103-127`'s dispatch);
`session_journal.rs` (`session-command`, redacted, before spawn);
`session_grants.rs` untouched apart from the family. Tests:
`the_host_prompt_never_prints_a_sandbox_line` (the trap: a naive port of
`run_program`'s facts would print one);
`the_no_sandbox_line_names_user_network_filesystem` (bytes pinned);
`a_grant_pre_answers_any_argv_of_the_program_and_re_asks_any_other_program`;
`interpreter_family_names_add_the_warning_and_command_tokens_never_interpreter_ones`;
`every_host_call_is_journalled_before_the_child_spawns` (ordering
sentinel);
`prefix_shaped_grants_refuse_at_parse`;
`run_program_s_prompt_gains_the_integrity_line_after_any_host_call`.
Gate: prompt snapshots per lane; every fact line traced to its enforcement
in review — the U5 gate rule (`UNIFY.md:604-609`).

**H3 — bypass, status, docs.** `session_activation.rs::bypass_line` gains
the lane fact when composed (bytes pinned; the uncomposed line keeps
today's bytes, `session_activation.rs:44-62`); status header gains a
`host:` segment (`docs/commands.md:58-61` moves with it);
`docs/commands.md`, `docs/configuration.md`, README, help strings, and
the `/allow` help. Tests:
`the_bypass_activation_line_states_the_lane_when_composed`;
`bypass_allows_every_host_ask_and_structural_refusals_still_refuse`
(lane-off, path-shaped, not-on-PATH, timeout); the help-parity suite
extends to the new token (the `scopes.rs:392-475` pattern). Gate: full
local gate (`AGENTS.md`) with the four surfaces agreeing on the new
words — flag, `/allow`, prompt suggestion, journal.

Each slice leaves the product working; H0 is pure addition; H1 is the only
slice that touches the grammar; H3 is copy and pins.

## 11. Uncertainties, one line each

1. Built env vs inherited is the decision I most expect to be argued;
   `pass_env` friction will tell — the honest widening is more names.
2. `run_command` naming is implementer's latitude; it must not reuse
   `run_program`, whose name carries the contained contract.
3. The 600s default is a guess; provisional in the `RUNNER_TIMEOUT_SECONDS`
   sense (`jobs.rs:50-54`) until measured.
4. Whether `session-command` events want an argv byte cap for journal size
   is open; redaction is required either way.
5. Linux needs no probe for this lane (no sandbox to prove), but the
   group-kill sweep is unverified there in CI terms; Windows stays
   fail-closed in v1.
6. Whether the lane should survive into `/run`-style nested sessions is
   not analysed — runs refuse it, so today the answer is trivially no.

Everything else here I derived from the code and the three prior decision
documents (UNIFY.md, SESSION-WORKSPACE.md, SCOPE-WIRING.md) and would
defend as stated.