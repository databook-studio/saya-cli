# ADR 0009: Native coding and continuity — instructions, preferences, guarded edits, recovery, and sessions that continue

- Status: accepted 2026-10-07. Records what shipped for release 0.4.2's
  native-coding milestone, verified against the code; where a plan and the
  code differ, the code is recorded. Numeric bounds are pinned by tests
  (see §Verification).
- Date: 2026-10-07
- Supersedes: nothing. Complements [ADR 0002](adr-0002-memory-and-contract-trust-model.md)
  (facts are never authority — instructions, preferences, notes, and
  continuations inherit that rule) and [ADR 0007](adr-0007-file-sources.md)
  (the workspace containment the file tools and checkpoints run inside).
- Records why coding in a project needs the agent to read your guidance and
  carry context across sessions without any of it becoming authority, why
  every file replacement is digest-guarded, and exactly what recovery data
  exists and what it never claims to undo.

## Context

saya answered database questions; coding in a project asks for three things
it did not have. The model needs the guidance you already wrote for agents
(`AGENTS.md`) and for yourself, loaded into each request as labelled,
untrusted context. Sessions need to survive their own length: a saved
summary, your notes, and observed file/check facts that carry across
`/resume` and `/rotate` without becoming a second memory system. And the
file tools need to be honest about replacement — overwriting a file you
changed underneath the agent destroys work — with an opt-in way to undo an
edit.

The design constraints come from what exists: every SQL statement still
crosses the read-only gate; stored guidance is context, never permission —
nothing an instruction, preference, note, or continuation says grants a
capability; the session record, the SQLite state store, and the config
layering already existed, and every new surface rides them rather than
introducing a parallel store or scheduler; and a terminal session still has
one human who can be asked, so recovery is opt-in per session and never
enabled silently.

## Decision

1. Instruction loading reads only workspace-root `AGENTS.md` and an optional personal
   `instructions.md` under the resolved user config directory — no recursive walk, imports,
   hooks, execution, or `CLAUDE.md` compatibility layer — and both are context, never authority.
2. Instruction caps are 32 KiB project, 16 KiB personal, 48 KiB aggregate, at most two files,
   personal before project, truncated at a UTF-8 boundary and marked, with source identity,
   digest, and truncation recorded in the context view.
3. The existing ContextBlock and request preflight remain the integration path: one CLI-owned
   projection assembles instructions, preferences, continuation, and observations into requests,
   and reuses unchanged sources by a cache key that includes source digest, workspace, profile,
   and privacy classification.
4. There is no automatic permanent memory: project preferences are explicit guidance the user
   confirmed, stored separately from typed database knowledge in the existing SQLite store,
   scoped to the canonical git worktree, and they never grant execution permissions.
5. Session continuation is an additive, explicitly versioned optional extension of the session
   record with a validated source-turn boundary and hash over redacted persisted data; a version
   this binary does not understand is preserved verbatim, never silently overwritten.
6. Continuation bounds are 32 KiB serialized, 8 KiB narrative, 8 KiB user notes, 64 file
   references, and 32 recorded checks; file and check facts are deterministic records of actual
   tool operations with provenance, and model prose cannot certify a test or a query.
7. Continuation is redacted before disk and revalidated before model inclusion: no raw query
   cells, complete arbitrary tool outputs, credential values, or model reasoning are ever
   persisted in it.
8. Replacing an existing file through the agent edit/write tools requires an observed
   `expected_digest` (the sha256 from a fresh read or the previous write/edit result), create is
   explicitly create-if-absent, append guards carry the expected digest and offset, and one
   per-path admission/commit policy is reused by every write-shaped adapter, including guarded
   removal.
9. These are optimistic stale-version guards, not a portable atomic filesystem compare-and-swap:
   saya's writers serialize, the anchored target identity/content is compared immediately before
   publication, and detected changes are refused — the guard does not promise protection from
   every non-cooperating external writer racing the final check.
10. Checkpoints cover direct file-tool edits only — never a subprocess, network, scratch, or
    production-database effect — restore runs only when the file still is, digest for digest, the
    recorded edit's result, and existing human changes are part of the baseline, never discarded.
11. Before-images are opt-in private recovery data outside the project that preserve exact
    bytes; known credential paths, secret-shaped content, and database-export artifacts are
    refused at admission (recovery-unavailable is surfaced, never silently saved), recovery is
    explained when enabled, and release artifacts are scanned so recovery data is never packaged.
12. Recovery quotas are 8 MiB per file, 64 MiB per session, and 128 edits per session; at a
    quota a recovery-required edit is refused before mutation, and checkpoint records distinguish
    pending, committed, and uncertain outcomes so a crash never masquerades as a successful
    checkpoint.
13. A session nearing its record cap rotates explicitly (`/rotate`) into a successor carrying a
    bounded validated continuation and a lineage-only parent link — publish the successor, then
    update selection, recoverable across crashes — and nothing is ever silently pruned.
14. Session deletion is coordinated by the harness's session lock with a stable tombstone
    outside the removable tree, checked before and after lock acquisition, refusing a live or
    current session, and cleaning the session JSON, its state directory, and its checkpoints
    together — while project preferences and typed database facts are named as remaining.
15. Sessions without a continuation keep their current behaviour; a continuation this binary
    does not understand is preserved and cannot be destroyed by a reader, and downgrade writes
    are unsupported without restoring a pre-upgrade backup.
16. `config doctor` reports independent database, AI, and coding readiness with a truthful
    configuration-readiness exit (`--task query|ai|coding`) — configuration readiness is never
    equated with successful network authentication — and the first screen offers coding in this
    project, a database, a data file, or the demo.
17. Provider setup may offer an already-present environment variable by name, bound to the
    selected provider and endpoint, and never enumerates or prints secret values, auto-sends a
    credential to a custom compatible endpoint, or loads `.env` implicitly.
18. Scripted coding uses the narrow headless `--turn-file` entry: no prompt on noninteractive
    runs — absent permission yields a typed refusal — and every scripted run ends with exactly
    one machine-readable `turn_result`.
19. `/reload` constructs a validated candidate runtime and applies it atomically between turns;
    a failed reload preserves the old configuration, and authority-sensitive settings never
    change for a running session.

### Chart saves and downloads share the replacement discipline

Decisions 8 and 9 extend past the file tools: `render_chart`'s optional
`save_to` and `http_download`'s destination are create-if-absent by
default, replacing an existing target only when the call carries the
`expected_digest` of its current content, as a fresh read reports it. The
chart tool (`crates/saya-cli/src/agent/tools/database_tools/chart_save.rs`)
refuses an existing file before anything is changed or opened; the download
guard (`crates/saya-harness/src/fetch/download_guard.rs`, workspace-side
check in `crates/saya-harness/src/workspace/download_guard.rs`) refuses an
existing destination before any request (best effort) and re-checks the
target at publication under the commit gate — stale-version checks
serialized among saya's own writers, never an operating-system
compare-and-swap. A refused download leaves the destination, its completed
partial, and the sidecar in place so a retry with the right precondition
does not re-download; the sidecar never grants permission to overwrite, and
grants last for the run (`--resume` does not restore them), so nothing
carried across sessions confers overwrite authority. Recovery is not
extended to these artifacts — `/recovery` still covers exactly
`workspace_write` and `workspace_edit` — and headless grant families
(`crates/saya-cli/src/interactive/session_grant_families.rs`) make only the
covered family available while per-call authority stays in the approval
engine.

## What shipped

| Capability | Surface | Where the shared operation lives |
| --- | --- | --- |
| Bounded instruction sources (root `AGENTS.md`, personal `instructions.md`) | every provider request; `/context` | loader `crates/saya-harness/src/instructions.rs`; settings `crates/saya-config/src/instructions.rs` |
| Projection into requests (labels, budget, cache key) | every provider request | `crates/saya-cli/src/agent/context_sources/mod.rs` |
| `/context` receipts — names, sizes, digests, never text | `/context` (slash) | `crates/saya-cli/src/interactive/session_context.rs` |
| Project preferences | `saya preferences …`, `/preferences …` | contract `crates/saya-types/src/project_preference/mod.rs`; store `crates/saya-store/src/project_preferences/mod.rs`; operations `crates/saya-cli/src/commands/preferences/operations.rs` |
| Versioned continuation in the session record | `/resume`, saves | contract `crates/saya-types/src/continuation/mod.rs`; persistence `crates/saya-store/src/continuation.rs` |
| Continuation save/restore, carried at rotation | `/compact`, `/resume`, `/rotate` | `crates/saya-cli/src/interactive/session_continuation.rs` |
| Deterministic file/check facts from tool activity | saved record | `crates/saya-cli/src/interactive/session_continuation/record.rs` |
| Session notes | `/notes [set\|clear]` | `crates/saya-cli/src/interactive/session_notes.rs` |
| Rotation successor + parent link | `/rotate` | `crates/saya-cli/src/interactive/session_rotate.rs`; `crates/saya-store/src/session_lifecycle/successor.rs` |
| Sessions list/delete, tombstones, lock coordination | `saya sessions list\|delete`, `/sessions list\|delete` | store `crates/saya-store/src/session_lifecycle/delete.rs`, `crates/saya-store/src/session_lifecycle/tombstone.rs`; lock `crates/saya-harness/src/session_lifecycle.rs`; CLI `crates/saya-cli/src/commands/saved_sessions/operations.rs` |
| Capacity report and near-limit warning | every save | `crates/saya-store/src/session_lifecycle.rs` |
| Digest-guarded create/replace/append, guarded removal | `workspace_write`, `workspace_edit` | harness `crates/saya-harness/src/workspace/guarded_write.rs`, `crates/saya-harness/src/workspace/patch.rs`, `crates/saya-harness/src/workspace/commit_gate.rs`; adapters `crates/saya-cli/src/agent/tools/database_tools/workspace_write.rs`, `crates/saya-cli/src/agent/tools/database_tools/workspace_edit.rs` |
| Checkpoint capture around direct edits | file tools behind `/recovery on` | `crates/saya-cli/src/agent/tools/database_tools/checkpoint/mod.rs` |
| Before-image storage and admission screening | `<sessions>/<id>/checkpoints/` | `crates/saya-store/src/checkpoints/ops.rs`, `crates/saya-store/src/checkpoints/screen.rs` |
| Recovery list/diff/restore | `/recovery [on\|off\|list\|diff\|restore]` | `crates/saya-cli/src/commands/recovery/ops.rs` |
| Turn cancellation through fetch/scratch tools | Esc/Ctrl-C in a turn | harness `crates/saya-harness/src/fetch/adapter.rs`, `crates/saya-harness/src/fetch/download.rs`, `crates/saya-harness/src/scratch/tools.rs`, `crates/saya-harness/src/scratch/import.rs`, `crates/saya-harness/src/scratch/execute.rs`; forwarding `crates/saya-cli/src/agent/tools/run_tools.rs` |
| Scripted turn result and tool tally | `--turn-file` | `crates/saya-cli/src/interactive/turn_summary.rs` |
| Configuration reload | `/reload` | `crates/saya-cli/src/config/reload.rs` (`reload/plan.rs`, `reload/classify.rs`, `reload/report.rs`) |
| `/setup` in the TUI, applied immediately | `/setup` (full-screen TUI) | `crates/saya-cli/src/setup/flow.rs`, `crates/saya-cli/src/setup/flow_commit.rs` |
| Per-task doctor readiness | `saya config doctor --task query\|ai\|coding` | `crates/saya-cli/src/config/doctor_task.rs` |
| Config source explanation | `saya config show --explain` | `crates/saya-cli/src/config/explain.rs`; provenance `crates/saya-config/src/provenance.rs` |

## Bounds

| Bound | Value | Constant (where it lives) |
| --- | --- | --- |
| Project instruction file | 32768 B | `DEFAULT_PROJECT_INSTRUCTION_BYTES` in `crates/saya-config/src/instructions.rs` |
| Personal instruction file | 16384 B | `DEFAULT_PERSONAL_INSTRUCTION_BYTES` in `crates/saya-config/src/instructions.rs` |
| Instruction aggregate | 49152 B | `DEFAULT_TOTAL_INSTRUCTION_BYTES` in `crates/saya-config/src/instructions.rs` |
| Continuation record | 32768 B | `MAX_CONTINUATION_BYTES` in `crates/saya-types/src/continuation/mod.rs` |
| Compaction narrative | 8192 B | `MAX_NARRATIVE_BYTES` in `crates/saya-types/src/continuation/mod.rs` |
| User notes | 8192 B | `MAX_USER_NOTES_BYTES` in `crates/saya-types/src/continuation/mod.rs` |
| File facts per continuation | 64 | `MAX_FILE_FACTS` in `crates/saya-types/src/continuation/mod.rs` |
| Check facts per continuation | 32 | `MAX_CHECK_FACTS` in `crates/saya-types/src/continuation/mod.rs` |
| Preferences per worktree | 64 | `MAX_PROJECT_PREFERENCES` in `crates/saya-types/src/project_preference/mod.rs` |
| Preference text | 1024 B | `MAX_PREFERENCE_TEXT_BYTES` in `crates/saya-types/src/project_preference/mod.rs` |
| Preference recall aggregate | 16384 B | `MAX_PREFERENCE_RECALL_BYTES` in `crates/saya-types/src/project_preference/mod.rs` |
| Session record cap | 4194304 B | `MAX_SESSION_BYTES` in `crates/saya-store/src/filesystem.rs` |
| Session record warning | 3145728 B | `SESSION_WARN_BYTES` in `crates/saya-store/src/session_lifecycle.rs` |
| Checkpoint file image | 8388608 B | `MAX_CHECKPOINT_FILE_BYTES` in `crates/saya-types/src/edit_checkpoint/mod.rs` |
| Checkpoint session quota | 67108864 B | `MAX_CHECKPOINT_SESSION_BYTES` in `crates/saya-types/src/edit_checkpoint/mod.rs` |
| Checkpoints per session | 128 | `MAX_CHECKPOINTS_PER_SESSION` in `crates/saya-types/src/edit_checkpoint/mod.rs` |
| Refreshed file facts per request | 16 | `MAX_REFRESHED_FILES`, private, in `crates/saya-cli/src/interactive/session_continuation/record_block.rs` |
| Recovery-list digest refreshes | 32 | `REFRESH_BOUND`, private, in `crates/saya-cli/src/commands/recovery/ops.rs` |
| Diff input | 65536 B | `DIFF_MAX_BYTES`, private, in `crates/saya-cli/src/commands/recovery/line_diff.rs` |
| Diff lines | 2000 | `DIFF_MAX_LINES`, private, in `crates/saya-cli/src/commands/recovery/line_diff.rs` |
| Diff table cells | 4000000 | `DIFF_MAX_CELLS`, private, in `crates/saya-cli/src/commands/recovery/line_diff.rs` |
| Diff output lines | 200 | `DIFF_MAX_OUTPUT_LINES`, private, in `crates/saya-cli/src/commands/recovery/hunks.rs` |
| Tool tally entries | 8 | `UNFINISHED_ENTRIES`, private, in `crates/saya-cli/src/interactive/turn_summary.rs` |
| Tool tally entry chars | 120 | `ENTRY_CHARS`, private, in `crates/saya-cli/src/interactive/turn_summary.rs` |
| Tool write/edit content | 65536 B | `WORKSPACE_WRITE_MAX_BYTES`, private, in `crates/saya-cli/src/agent/tools/database_tools/workspace_write.rs` |
| Patch replacement block | 65536 B | `PATCH_REPLACEMENT_MAX_BYTES` in `crates/saya-harness/src/workspace/patch.rs` |
| Workspace I/O ceiling | 8388608 B | `MAX_IO_BYTES` in `crates/saya-harness/src/workspace/contain.rs` |

Constants marked private cannot be imported by another crate's test; each is
guarded by `saya-cli`'s own unit tests (`turn_summary_tests.rs`,
`recovery/ops_tests.rs`, `session_continuation/record_block_tests.rs`,
`recovery/diff_tests.rs`, `workspace_write_tests.rs`), and the `pub`
constants are pinned against this table in each owning crate's
`tests/adr_bounds.rs`.

## Data and privacy promises

| Data | Stored where | Deleted when | Reaches the model |
| --- | --- | --- | --- |
| Session record (redacted turns, settings, pinned workspace root) | one JSON file under the sessions root (`SAYA_SESSION_DIR`, else the platform data directory) | only by `saya sessions delete` / `/sessions delete`; never pruned or truncated | redacted turns return to provider history on resume, under the existing privacy classes |
| Continuation (summary, notes, file/check facts) | inside the session record, `continuation` field | with the session; a `/clear` drops a current one | redacted before disk, revalidated before inclusion, sent as labelled untrusted context |
| Session notes | inside the continuation, verbatim user text | `/notes clear`, or deleting the session | sent as untrusted guidance; the model never writes them |
| Project preferences | state DB `project_preferences` table (`SAYA_STATE_DB`), keyed by canonical worktree | explicit `forget`/`forget-all` only; never auto-created, never model-written | sent per request as untrusted guidance, up to the recall cap |
| Checkpoint before-images | `<sessions>/<id>/checkpoints/`, outside the project, `0600`/`0700` | consumed by a successful restore; removed with the session | never |
| Tombstone | sessions root sibling file, outside the removable tree | removed when cleanup converges | never |

Never stored, anywhere in the above: secret values (only `env:`/`file:`
references), raw database cell values, complete arbitrary tool outputs,
credential-shaped content (refused, not redacted, at preference/note/
checkpoint admission), model reasoning, or session narrative outside the
session record. Instruction files are not stored at all — they are read
anew, bounded, on every request.

## Recovery coverage and limits

`/recovery on` is opt-in per session, off by default; the flag is not
persisted, so a resumed session starts off again. While it is on,
`workspace_write` and `workspace_edit` keep the file's exact previous bytes
as a before-image under `<sessions>/<id>/checkpoints/` — private recovery
data, outside the project, separate from the session narrative and from
database evidence. Coverage is exactly the two file tools: a restore never
undoes a command, a download, a scratch table, or a database effect, and it
never resets a git checkout. Restore runs only when the file still is,
digest for digest, the recorded edit's result; after a successful restore
the copy is consumed.

Admission screening refuses credential-looking paths (`.ssh`, `.aws`,
`.kube`, `.gnupg`, `.docker`, `.saya` components and credential file names),
secret-shaped content, and database-export artifacts, and says
`recovery-unavailable` instead of silently storing prohibited bytes — but
screening is a refusal table, not a proof that arbitrary source text
contains no sensitive information. Quotas: 8 MiB per file image, 64 MiB and
128 edits per session; at a quota an edit that would need recovery is
refused before anything is mutated, and the user can delete checkpoints
(with the session, via `/sessions delete`) or turn recovery off for later
edits. Around the commit the capture is best-effort: prepare stores the
image before the edit is known to land, a crash in the window leaves an
`Uncertain` record (converted from `Prepared` by the recovery pass on
acquire, never auto-restored and never silently committed), and a snapshot
failure never masquerades as a successful checkpoint.

## Rotation and capacity

A session record is capped at 4 MiB; the store warns once at 3 MiB, and a
save at the cap is refused with the last good record left untouched —
nothing is ever truncated or pruned to fit. `/rotate` (after `/compact`)
saves the current session, publishes the successor linked to it by a
lineage-only parent link, and continues there, carrying the saved summary,
the user's notes, the file/check facts, and the not-yet-summarised verbatim
tail; grants, approvals, denials, and the recovery setting are not
inherited. Rotation is publish-successor-then-update-selection, so a crash
between the two leaves a resumable successor and the old session untouched;
an oversized successor is refused rather than saved.

## Reload semantics

`/reload` re-reads the same files with the same launch options, builds a
candidate runtime, and applies it atomically between turns; an invalid
candidate changes nothing and says the previous configuration is still in
use. The report names three classes: **applied from the next turn** (settings
the agent re-reads per turn — row caps, timeouts, budgets, connections),
**updated for this session** (provider, model, default profile, endpoint
re-seeded now — unless the user overrode one with `/provider`, `/model`, or
`/connect`, which is named as *not applied*), and **not applied in this
session** (data sharing, host-command shaping, session denies, the jobs
runner, theme, and thinking display — all take effect in a new session).
Grants and the approval mode are untouched. Authority-sensitive settings
never change for a running session; privacy changes go through `/privacy`.

## Downgrade behaviour

The state database is SQLite with `PRAGMA user_version` 9; a binary older
than this refuses a database written by a newer one
(`StoreError::VersionUnsupported`) instead of guessing at its schema. The
session-record format `SESSION_VERSION` is 2, and every new field
(`continuation`, `parent`, `task_list`, `agent_mode`, `workspace_root`) is
additive with `#[serde(default)]`, so a record written before a field
existed loads with the field's default and no version bump. An older binary
reading a newer session record ignores the fields it does not know — and
if it rewrites the record it **drops** them: continuation and the parent
link are lost. Downgrade writes are unsupported without restoring a
pre-upgrade backup. This binary, for its part, preserves what it does not
understand: a continuation with an unknown version loads as
`StoredContinuation::Unsupported` verbatim, `/clear` drops a current
continuation but preserves an unsupported one and the parent link, and a
save that would overwrite an unsupported continuation is refused
(`StoreError::UnsupportedContinuation`) before any disk write.

## Platform notes

The containment, guarded-write, and checkpoint machinery is anchored unix
code: no-follow opens with post-open identity checks, `0600` files in
`0700` directories, atomic private-temp renames, and no-follow removals.
It is verified where the acceptance suites ran — macOS, locally, across
`tests/turn_file_coding`, `mixed_task`, `journey`, `instruction_examples`,
`reload_repl`, and the `tui_pty_smoke` setup/reload tests. Windows builds a
non-unix by-path guarded-write fallback that compiles in CI but was not
exercised by these acceptance tests; its behaviour is not claimed beyond
"it compiles".

## Known limits

- Headless `ask` and read-only sessions advertise no write-shaped tools;
  headless coding needs `--approval-mode bypass` (and inherits bypass's
  stated residual risks).
- A session record over 4 MiB cannot be loaded — the bounded read refuses
  at the cap. The 3 MiB notice and `/rotate` exist to keep sessions under
  it; the last good record stays resumable.
- A resumed session keeps its persisted provider endpoint: a changed
  `[ai] base_url` does not move a resumed session; `/provider` or a new
  session does.
- `/reload` never changes data sharing, host commands, session denies, the
  jobs runner, theme, or thinking display in a running session.
- `config.toml`'s `[ui] theme` has no effect today — the `--theme` flag's
  default always wins. This is a known 0.4.1 defect, not fixed in 0.4.2;
  `--theme dark|light` works.
- `--no-color` is applied after colour resolution, without provenance.
- SIGTERM terminates a scripted run immediately, without a `turn_result`;
  send SIGINT to cancel cleanly and get one.
- Checkpoint capture is best-effort around the commit: a crash inside the
  window leaves an `Uncertain` record that is never auto-restored.
- The file tools take no cancellation context: Ctrl-C during a write lets
  the atomic write finish, and the call is listed under `tools_unfinished`.
- `saya-config` provenance records no file paths; the paths in
  `config show --explain` are derived at render time.
- Admission screening refuses known credential-shaped material but is not a
  proof that arbitrary recovered source text contains no secrets.
- Windows: the non-unix by-path guarded-write implementation compiles in CI
  but was not exercised by the acceptance tests (macOS only).

## Verification

The claims above are exercised by the real-binary acceptance suites in
`crates/saya-cli/tests/`: `turn_file_coding` (headless edit/test loops),
`mixed_task` (mixed database-and-code privacy boundaries), `journey`
(the full project journey), `instruction_examples` (the copy-ready
instruction files and the `/context` sample), `reload_repl` (reload
between turns), and `tui_pty_smoke` (the `/setup` handoff and reload
tests). The bounds table is pinned by `docs_claims.rs` and each owning
crate's `tests/adr_bounds.rs`; store-side lifecycle, continuation,
checkpoint, and screening behaviour is pinned in `crates/saya-store/tests/`
(`session_lifecycle.rs`, `sessions.rs`, `checkpoints.rs`,
`project_preferences.rs`).