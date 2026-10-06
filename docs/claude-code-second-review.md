# Saya 0.4.2 architecture and coding experience review

Reviewed 5 October 2026. Saya release worktree: `release/v0.4.2` at `faa0a8c875ad2640eb5c2cdd5fee0c0f9161f65f`. Comparison mirror: `6f6f12b37f529488b10e53928dd5508bb93535c7`, the same snapshot used in the September review. Current official Claude Code documentation was checked separately.

**Recommendation:** develop Saya as a native coding assistant with database expertise by completing its existing harness. The largest remaining gaps are project context, continuity across restarts, recoverable edits, consistent execution across interfaces, and task-aware onboarding. A replacement agent framework would duplicate substantial work already present.

This is a source-based architectural and usability review. Three external GLM research packets were independently audited; incorrect preliminary claims were discarded after tracing runtime composition. No product code or tests were changed, no provider/database was contacted, and no fresh runtime, performance, or release-gate result is claimed. Prior recorded test counts are not current validation. The reference mirror describes missing components and stubs; its source is comparison material, not authenticated evidence of current production behavior. Recommendations below describe independent changes to Saya, not importing the mirror's implementation. [Reference snapshot](https://github.com/tanbiralam/claude-code/tree/6f6f12b37f529488b10e53928dd5508bb93535c7).

## What the previous work has achieved

Do not reopen these as missing features:

| Earlier concern | Current source evidence | Qualification |
| --- | --- | --- |
| Shared permission decisions | `saya-agent/src/loop_runner/tool_policy.rs:72`, consumed by sequential and concurrent dispatch in `turn_tools.rs` | Keep structural restrictions separate from consent |
| Concurrency distinct from permission | `saya-agent/src/protocol/contracts/tool.rs:56`, serial default | A permitted call is not automatically safe to parallelize |
| Finite interactive budgets | `saya-config/src/investigation_budget.rs:24`; `saya-cli/src/agent/runtime.rs:590` | Defaults are 24 answering requests, 64 requested tool calls, 300 seconds, 250,000 known reported tokens; not a total billing ceiling for all auxiliary work |
| Request context admission | `saya-cli/src/agent/runtime.rs:602` wires request preflight into run control | Do not infer absence from legacy `AgentLimits` defaults |
| Agent SQL cancellation | `saya-cli/src/agent/state_tools.rs:395` → `state_tools/operation.rs:105` | Requests connector cancellation and waits for execution settlement; this does not prove remote cancellation on every engine |
| Automatic compaction beyond TUI | `saya-cli/src/interactive/session_auto_compact.rs:33`; `session_loop.rs:651` | Restart persistence remains a separate gap |
| Typed recovery and durable runs | `saya-agent/src/protocol/contracts/recovery.rs:23`; `saya-harness/src/engine/resume.rs` | Durable runs and interactive conversation records have different authority rules |
| Artifact inspection | `scripts/check-artifact-content.py`; `scripts/check-release-workflow.sh:23` | Gate implementation exists; artifacts were not rebuilt or scanned in this review |
| Setup and exploration | `saya-cli/src/setup/flow.rs:71`; `setup/prompt_database.rs:40`; `cli.rs:173` | Guided setup can skip the database; demo already requires no provider |

Local phase metrics also exist in the shared database operation (`state_tools/operation.rs:26–45`). Evidence receipts and broader metrics coverage were not exhaustively re-audited. The main checkout's staged `docs/standards/architecture.md` still omits the harness, but that file is untracked guidance, not evidence of a missing tracked release implementation.

## Findings and priorities

### 1. Make cancellation consistent across tool families

**Priority: release hardening.** `RunTools::execute_with_context` forwards context to database tools, but calls the context-free scratch and fetch executors. Their operations cannot observe the turn token through this interface. A user cancelling a download or scratch operation may therefore wait for its existing timeout or completion.

Evidence: [composite dispatch](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/agent/tools/run_tools.rs:230), [fetch executor](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-harness/src/fetch/adapter.rs:135), [scratch executor](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-harness/src/scratch/tools.rs:231).

Thread the existing context into these members. Define settlement for blocking scratch work and partial downloads; simply dropping an async future is insufficient when a blocking task or file effect continues. Preserve atomic publication and account for consumed work. Verify cancellation during transfer/import and the absence of subsequent effects.

The host and sandboxed runner do receive the current turn token: `runtime.rs:583` passes it to `SessionUniverse::executor`, and `session_universe.rs:428–454` binds it into both executors. There is no supported finding of a host-token mismatch.

### 2. Protect edits from stale context and add reviewable recovery

**Priority: before broader autonomous coding.** Saya already supports read/list/grep/glob, anchored edits, chunked appends, and whole-file writes. However, `workspace_edit` makes `expected_size` and `expected_digest` optional and explicitly documents last-writer-wins behavior when guards are absent. An agent can edit against stale context while a person or another tool changes the same file.

Evidence: [edit contract](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/agent/tools/database_tools/definitions.rs:276), especially lines 288–291 and the optional schema fields at 324–331.

Require a digest from an observed version for edits to existing files; use explicit create-if-absent semantics for new files. Surface a useful stale-file error that prompts a reread. Build a changed-files view and bounded before-images for direct Saya file edits, allowing restore only when the current digest still matches Saya's last write. Preserve pre-existing dirty changes. Keep this inside existing workspace/session storage owners.

A journal records execution; it is not proof of file rollback. Checkpoint coverage must be explicit: direct file edits first, no claim that a checkpoint reverses arbitrary commands, network effects, or database operations. Claude Code's own documented rewind excludes Bash-created changes. [Checkpoint limitations](https://code.claude.com/docs/en/checkpointing).

### 3. Add automatic project instructions

**Priority: first coding capability increment.** Saya's reviewed system assembly combines connection facts, memory guidance, session facts, working guidance, and answer conventions. Research found no automatic project-instruction loader in that assembly. Tools can read a file when requested, but project conventions do not reliably enter every session.

Evidence: [prompt assembly](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/agent/system_prompt.rs:181). This is an automatic-loading gap, not a claim that Saya cannot read Markdown.

Start with existing `AGENTS.md` at the bound workspace root and one optional personal instruction file. Avoid requiring another proprietary project filename. Add subtree rules only after the root workflow works. Establish deterministic precedence, workspace containment, file/aggregate byte caps, source paths and content hashes, and visible truncation. Include loaded instruction sources in a context view. Never let instruction text grant process, network, database, or filesystem authority.

The useful Claude Code pattern is persistent project guidance loaded independently of conversation history. Its official documentation separates instructions from enforced client policy, and describes project instructions and automatically maintained memory as distinct features. [Project instructions and memory](https://code.claude.com/docs/en/memory).

### 4. Preserve working context across restart without retaining raw tool output

**Priority: first continuity increment.** Interactive `compaction_summary` and `compacted_turns` are explicitly skipped during serialization. Resume reconstructs the persisted conversation, but not the compacted working summary. Request preflight still applies, so this does not mean the entire old history necessarily reaches the provider; it means the useful compressed continuation is lost and must be rebuilt or replaced by a truncated history.

Evidence: [in-memory summary fields](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_state.rs:57), [persisted session shape](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-store/src/lib.rs:60), [resume](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_resume.rs:45).

Persist a versioned, bounded continuation record: goal, open tasks, changed-file references, last observed checks, unresolved questions, source-turn boundary, and summary. Store deterministic facts separately from model narrative. Bind it to workspace, source turns, profile/privacy scope, and schema identity where relevant. Redact before writing and recheck disclosure policy before replay. Prefer stable turn identifiers or a validated source boundary over a bare count after privacy filtering.

Do not simply serialize today's summary field: it can contain sensitive SQL literals, identifiers, or prose. Keep raw query cells, arbitrary process output, credentials, and internal model reasoning out. On resume, re-read file digests and label historical checks with their revision/time. Neither a summary nor notes restore grants or certify current tests.

There is also a storage limit issue: turns accumulate, while compaction does not prune the durable record. `FsSessionStore` refuses serialized sessions over 4 MiB, preserving the previous saved version. Long-running sessions can outgrow saving even after successful context compaction. Add visible near-limit guidance and an explicit continuation/rotation path; do not silently delete history or raise the bound indefinitely. Research found no session deletion/retention command in the reviewed command paths.

Evidence: [session cap and save](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-store/src/filesystem.rs:66), [compaction apply](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_compact.rs:98).

### 5. Keep project memory, session notes, and database knowledge distinct

**Priority: same continuity increment.** Existing memory is structured database knowledge with scoped claims, confirmation states, and evidence handling. The persisted task list already tracks work. These should remain useful foundations; they do not yet provide general coding preferences or durable narrative session notes.

Use three small concepts:

| Context | Contents | Trust and persistence |
| --- | --- | --- |
| Project instructions | User/team conventions, build commands, architecture guidance | User-editable source file; bounded context; no permission grants |
| Session continuation | Current goal, decisions, changed files, next actions, historical checks | Private session record; generated narrative marked as such; atomic save and deletion |
| Long-lived memory | Explicitly accepted project preferences; existing database claims | Project preferences separate from profile/schema-bound database facts; provenance and forget support |

Make remembering inspectable and reversible. Begin with explicit remember/edit/forget operations for general project preferences. Add automatic suggestions only after evidence that they reduce repetition; do not silently turn every conversation into permanent memory. Keep project/worktree/profile scoping explicit. Deleting a session should explain whether independent learned facts remain.

The mirror has a bounded memory entry point and a separate session-note mechanism. Borrow that separation, not its exact prompts or token constants. [Memory entry point](https://github.com/tanbiralam/claude-code/blob/6f6f12b37f529488b10e53928dd5508bb93535c7/src/memdir/memdir.ts#L34-L55), [session-note mechanism](https://github.com/tanbiralam/claude-code/blob/6f6f12b37f529488b10e53928dd5508bb93535c7/src/services/SessionMemory/prompts.ts#L8-L41).

Avoid attaching all new notes work to the database learning extractor. That extractor intentionally has no total wall-clock deadline, although transport/retry/output limits exist (`agent/learning/post_turn.rs:90–110`). Use deterministic local state first; optional model summarization needs its own bounded, cancellable budget and should not delay the user's next action.

### 6. Present coding as a supported starting point

**Priority: small release usability work.** A provider-only setup is supported: the database menu has a skip option. A workspace session can use coding tools without a database profile. Yet the empty state centers on “No database is configured yet” and lists database setup steps. It makes a valid coding setup look unfinished.

Evidence: [setup flow](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/setup/flow.rs:71), [empty state](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/tui/ui/surface/splash.rs:10), [session composition](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_universe.rs:190).

On first launch offer a short choice: work on this project, connect a database, open a data file, or try the demo. Reuse guided setup, with database setup optional and a direct return to the session. Show independent status for AI, workspace, and database. Expose existing `/provider` and `/model` switches instead of implying every switch needs restart. `saya open` is the CSV/Parquet analytics entry, not the coding entry.

Make `config doctor` task-aware. Today its exit code depends on a selected database profile and secret resolution, not AI readiness. A provider-only coding setup gets exit 3; a database-ready setup with missing AI credentials can get exit 0. This is coherent for its narrow query check but misleading as general startup guidance. Add distinct readiness results and a task-specific exit contract; distinguish configuration checks from optional live probes.

Evidence: [doctor status](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/config/doctor.rs:11), [exit predicate](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/config/doctor.rs:108), [existing model/provider switches](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_commands.rs:100).

Setup already suggests standard provider key environment names and reports whether they exist. Reduce repetitive entry with an explicit “use this existing key reference” action tied to the chosen endpoint. Never silently send a standard provider key to a custom compatible endpoint. Offer supported OS credential storage later: the current `SecretRef::Keyring` path returns `KeyringUnavailable`; do not advertise a working keychain today. Browser login is worth adding only through a provider-supported flow. [Secret resolver](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-config/src/secret.rs:39), [provider setup](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/setup/prompt_provider.rs:50), [Claude Code first-run authentication](https://code.claude.com/docs/en/quickstart).

### 7. Unify coding execution across interfaces without silently widening permissions

**Priority: after interactive continuity.** Interactive sessions compose workspace, task, scratch, fetch, and process capabilities. One-shot `ask` uses the bare database executor and rejects `--workspace`. The refusal is honest, but users cannot take the same coding task from an interactive session into a simple scripted invocation.

Evidence: [runtime executor selection](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/agent/runtime.rs:582), [workspace flag guard](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/app.rs:195).

Provide one supported noninteractive coding entry backed by the same application operations and workspace composition. Decide whether to extend `ask` or expose a session print mode before implementing either. Existing headless session/turn-file paths may provide the shortest route. Keep explicit permissions and fail-closed prompts when no approval surface exists; parity means shared behavior, not automatic host access. Preserve the current database-oriented `ask` contract until a deliberate migration is specified.

## Architecture to build toward

Keep the current Rust workspace. Improve ownership incrementally when a feature touches a boundary:

| Owner | Responsibility |
| --- | --- |
| `saya-agent` | Provider protocol, turn execution, pure tool policy, context admission, retry/usage accounting |
| `saya-harness` | Workspace operations, processes/sandbox, scratch/fetch execution, durable runs, edit recovery mechanics |
| `saya-connectors` | Database connection and bounded read-only SQL safety |
| `saya-store` | Session/continuation/knowledge persistence, migrations, redaction, retention mechanics |
| `saya-config` | Configuration resolution, credential references, bounds, explainable winning configuration |
| `saya-types` or the owning contract crate | Shared data contracts without presentation or authority inferred from prose |
| `saya-cli` | Composition, shared application entry points, CLI/TUI/headless adapters, all presentation |

The current `DatabaseTools` definitions also contain workspace tools, and `RunTools` combines several capability families. Split by domain as those features change, retaining the existing executor interface. Do not introduce another generic dispatcher, event bus, plugin loader, or agent framework solely to make the graph look cleaner.

Use one session context projection over distinct durable sources: project instructions, permitted memory, continuation state, and current observations. Keep authority separate: runtime policy and approved run journal state determine actions. Interactive consent journals remain audit records rather than remembered grants.

Process execution needs an explicit product promise. `run_program` is the sandboxed lane. `run_command` is an unsandboxed host lane, composed in supported root-bound sessions and subject to the current approval policy; a workspace cwd does not confine filesystem or network access. Bypass can waive consent, not SQL safety or other structural checks. A bare program deny list is not transitive confinement of child programs.

Evidence: [current composition](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_universe.rs:243), [host execution description](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_definitions.rs:178), [sandboxed execution description](/Users/subodhsharma/Projects/saya-wt-rel/crates/saya-cli/src/interactive/session_definitions.rs:212).

Saya can guarantee its database tools use bounded read-only SQL. It cannot extend that promise to arbitrary approved host programs. Prefer contained execution for normal coding tasks, explain host access at the action that needs it, and keep database credentials out of default child environments. Package managers, test runners, and build scripts execute project code; granting their names is broader than granting one harmless command. Do not add a configuration shortcut that obscures this distinction.

## Proposed delivery sequence

This is a review and implementation proposal, not implemented behavior or a release certification. Each behavior slice should follow plan-first/TDD and the repository's test-audit gate, with an independent diff review.

| Order | Concrete outcome | Main owners | Acceptance evidence |
| --- | --- | --- | --- |
| A | Scratch/fetch cancellation reaches active work and settles safely | Harness, CLI composition | Cancel delayed transfer/import; no late publication or next effect; useful terminal outcome |
| B | Coding-aware first screen and doctor | CLI, config | Empty config, provider-only, database-only, missing-key scenarios through real dispatch/PTY |
| C | Project instructions enter context predictably | Harness/config, agent contract, CLI | Loaded-source receipt; workspace containment; malicious text cannot grant authority; bounded UTF-8 input |
| D | Durable continuation and session lifecycle | Store, CLI, agent context | Compact → restart → continue; changed files/privacy scope invalidate stale facts; size-limit rotation preserves the last good record |
| E | Stale edits refuse; direct edits can be reviewed and restored | Harness, store, CLI | Concurrent human edit survives; restore refuses changed current digest; new files and disk-full cases handled |
| F | Shared noninteractive coding operation | CLI, existing harness | Same synthetic task succeeds in supported interactive and scripted paths; missing authority yields an explicit refusal |
| G | Project preferences and reusable workflows | Knowledge/config, existing context path | Remember/inspect/forget round-trip; profile/worktree isolation; no accidental promotion of generated claims |

Keep A/B focused for 0.4.2. Put C–G in separately reviewable increments; expanding an already large release in one merge would make the safety and migration changes harder to assess. If coding parity is required for the 0.4.2 label, treat C–F as explicit release scope rather than claiming existing tools alone deliver it.

Start reusable workflows as bounded instruction templates for tasks such as fix-test, query-review, or dbt-model review. Add plugin discovery, hooks, LSP integration, background agents, or a broader skill system only after concrete tasks show the simpler path is insufficient. An optional official external coding-agent integration may be useful later, but Saya's native task loop should own its own operation lifecycle.

## Product acceptance scenarios

Use small synthetic repositories and databases; compare the same tasks, model, environment, and budgets. The following are proposals, not tests run during this review:

1. **First useful coding action:** empty config, no database; configure a provider, explain a local project, edit one file, run its test, display the diff. Measure elapsed time, prompts, config edits, and successful completion.
2. **Database work without AI:** `demo`, schema exploration and bounded SQL remain usable without provider credentials.
3. **Mixed code and data:** inspect a schema, repair application SQL or a dbt model, run local tests, validate with a bounded query; no production write and no unapproved row disclosure.
4. **Next-day continuation:** compact a long task, exit, alter a file externally, resume. Retain the goal and next steps, detect changed state, and avoid presenting old checks as current.
5. **Concurrent edit and recovery:** a human edits after Saya reads; Saya refuses stale replacement. Revert only its own unchanged direct edits, preserving unrelated dirty work.
6. **Cancellation:** cancel query, process, download, and scratch import separately; measure time to settlement and prove no subsequent effect is admitted.
7. **Untrusted context:** repository text, fetched content, database comments, and generated memory request broader permissions. Runtime authority remains unchanged.
8. **Privacy and lifecycle:** sentinel credentials/rows never enter prohibited persistence or outbound context; session deletion covers its artifacts and explains independent knowledge retention.

Measure task success, unnecessary approvals, stale edits prevented, resume fidelity, configuration effort, time to useful output, cancellation settlement, and reported token use. Report unknown usage as unknown. No comparative performance result is asserted here.

## Review limits

Source traces establish implementation structure and the listed gaps, not end-to-end coding quality. No live engines, supported-platform sandbox probes, provider matrix, UI usability study, or full local Rust gate was run. The mirror snapshot was read as data only. Current official documentation is cited for current advertised workflows; mirror observations are limited to the pinned snapshot. The earlier review remains historical; this report supersedes its missing-budget, missing-agent-cancellation, and TUI-only-auto-compaction findings for the paths verified above.
