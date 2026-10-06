# Native coding packet catalog

56 small packets. Status: planning only. Read [PLAN.md](PLAN.md), [DECISIONS.md](DECISIONS.md) and [VALIDATION.md](VALIDATION.md) before activating any packet. [TASKS.json](TASKS.json) is the machine-readable counterpart.

Every packet inherits the same non-negotiables: bounded read-only SQL; no authority from context; local-only changes; one writer per file/worktree; test-audit and meaningful red before behavior edits; no new framework/dependency or broad cleanup. The lead must inline the protocol, decisions and exact verified ownership into TASK.md before launch. Candidate NEW paths are design proposals, not claims that code already exists.

Each packet includes its tests. Follow the existing primary owner first; a proposed NEW test file is permitted only for a distinct boundary and must pass test-audit. Shared parser/dispatch edits include the corresponding SessionAction and headless/TUI arms; never implement a private helper while leaving real dispatch broken.

All implementation packets require the lead to inspect the actual diff, independently validate immutable code, and check all targets after the local merge. A passing worker report alone cannot close a packet. Research/verify-only work closes with reviewed evidence and never fabricates a code commit.

| ID | Outcome | Owner | Scope | Dependencies |
| --- | --- | --- | --- | --- |
| [A01](#a01) | Cancel bounded HTTP fetch | harness | release | B00 |
| [A02](#a02) | Cancel resumable downloads before publication | harness | release | A01 |
| [A03](#a03) | Interrupt and settle scratch queries | harness | release | B00 |
| [A04](#a04) | Cancel scratch imports transactionally | harness | release | A03 |
| [A05](#a05) | Wire cancellation through composite execution | cli | release | A02, A04 |
| [B00](#b00) | Freeze baseline and activate packet ownership | lead | foundation | None |
| [C01](#c01) | Resolve bounded instruction-source settings | config | core | B00 |
| [C02](#c02) | Read bounded root and personal instruction sources | harness | core | B00 |
| [C03](#c03) | Project instruction context into provider requests | cli | core | C01, C02 |
| [C04](#c04) | Show loaded context sources | cli | core | C03 |
| [D01](#d01) | Define versioned continuation records | contracts | core | B00 |
| [D02](#d02) | Persist continuation extensions without data loss | storage | core | D01 |
| [D03](#d03) | Build deterministic continuation facts | cli | core | D01 |
| [D04](#d04) | Save validated compaction continuation | cli | core | D02, D03 |
| [D05](#d05) | Restore scoped continuation on resume | cli | core | D04, C03 |
| [D06](#d06) | Report session capacity before the save ceiling | storage | core | D02 |
| [D07](#d07) | Tombstone and clean session storage retryably | storage | core | D06 |
| [D08](#d08) | Offer explicit session rotation | cli | core | D05, D11 |
| [D09](#d09) | Expose session deletion and retention inspection | cli | core | D12 |
| [D10](#d10) | Inspect and edit durable session notes | cli | core | D05 |
| [D11](#d11) | Publish an explicit successor session atomically | storage | core | D02, D06 |
| [D12](#d12) | Coordinate deletion with the live session lock | harness | core | D07 |
| [E01](#e01) | Verify patch and append digests inside workspace commit | harness | core | B00 |
| [E02](#e02) | Require observed digest for agent edits and appends | cli | core | E01 |
| [E03](#e03) | Close whole-file overwrite bypass | cli | core | E02, E09 |
| [E04](#e04) | Define checkpoint state and recovery policy | contracts | core | B00 |
| [E05](#e05) | Store bounded private checkpoint before-images | storage | core | E04 |
| [E06](#e06) | Capture checkpoints around direct file commits | cli | core | E03, E05 |
| [E07](#e07) | Restore only the last matching direct edit | harness | core | E06 |
| [E08](#e08) | Review changes and explicitly restore a checkpoint | cli | core | E07, D09 |
| [E09](#e09) | Guard whole-file create and replacement | harness | core | E01 |
| [H01](#h01) | Verify the existing scripted coding entry and permissions | cli | core | B00 |
| [H02](#h02) | Close only demonstrated turn-file coding gaps | cli | core | H01 |
| [H03](#h03) | Align scripted terminal outcomes and cancellation | cli | core | H02, A05 |
| [M01](#m01) | Define explicit worktree preference records | contracts | core | B00 |
| [M02](#m02) | Persist scoped preferences in existing store | storage | core | M01 |
| [M03](#m03) | Recall project preferences through context projection | knowledge | core | M02, C03 |
| [M04](#m04) | Expose explicit project preference operations | cli | core | M02 |
| [M05](#m05) | Ship reusable instruction workflow examples | release | follow-on | C04, M04 |
| [O01](#o01) | Track winning configuration origins | config | follow-on | C01 |
| [O02](#o02) | Explain effective configuration sources | cli | follow-on | O01, U02 |
| [O03](#o03) | Prepare atomic between-turn config reload | cli | follow-on | O02, H03 |
| [O04](#o04) | Expose explicit config reload | cli | follow-on | O03 |
| [Q01](#q01) | Prove the complete native coding journey | quality | acceptance | U05, U06, C04, D08, D10, E08, M04, H03, U02, U07, M03 |
| [Q02](#q02) | Prove mixed code and database privacy boundaries | quality | acceptance | A05, C04, D05, E03, M04, H03 |
| [Q03](#q03) | Independently audit integrated correctness and authority | quality | acceptance | Q01, Q02 |
| [Q04](#q04) | Publish accurate local capability and migration documentation | release | acceptance | Q03 |
| [Q05](#q05) | Run final local integration gates | lead | acceptance | Q04 |
| [Q06](#q06) | Audit real native release artifacts and retained state exclusion | release | acceptance | Q05 |
| [U01](#u01) | Compute task-specific configuration readiness | cli | release | B00 |
| [U02](#u02) | Expose task-aware doctor command | cli | release | U01 |
| [U03](#u03) | Show coding readiness without a database | cli | release | U01 |
| [U04](#u04) | Reuse guided setup for selected startup intent | cli | release | B00 |
| [U05](#u05) | Hand terminal ownership to guided setup safely | cli | release | U04 |
| [U06](#u06) | Clarify endpoint-bound credential reference selection | cli | release | U04 |
| [U07](#u07) | Wire startup choices to shared setup operations | cli | release | U03, U05 |

## A01

**Cancel bounded HTTP fetch**

Owner: `harness` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Carry the existing turn token through fetch admission, DNS/redirect/request/body waits; return typed cancellation and retain existing byte/time limits.

**Dependencies:** B00.

**Exclusive leases:** fetch. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/fetch/adapter.rs` — existing
- `crates/saya-harness/src/fetch/adapter_tests.rs` — existing
- `crates/saya-harness/src/fetch/tools.rs` — existing

**Red or verification target:** A held local fake transport is cancelled while awaiting headers/body; fetch settles cancelled and consumes no next hop. Extend existing transport-boundary tests, not a token predicate test.

**Evidence anchors:** `crates/saya-harness/src/fetch/tools.rs:111`; `crates/saya-harness/src/fetch/adapter.rs:135`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## A02

**Cancel resumable downloads before publication**

Owner: `harness` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Apply token-aware waits and a checked publication boundary to downloads. Preserve existing verified partial/resume behavior and consumed-byte accounting.

**Dependencies:** A01.

**Exclusive leases:** fetch. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/fetch/adapter.rs` — existing
- `crates/saya-harness/src/fetch/adapter_tests.rs` — existing
- `crates/saya-harness/src/fetch/download.rs` — existing
- `crates/saya-harness/src/fetch/download_error.rs` — existing

**Red or verification target:** Cancel after a body chunk: destination remains unchanged, partial is bounded/valid, explicit retry works without lost or double-counted bytes. Separately prove completion winning the commit boundary reports committed outcome.

**Evidence anchors:** `crates/saya-harness/src/fetch/download.rs:42`; `crates/saya-harness/src/workspace/download.rs:61`.

**Risk:** Cancellation must not delete a pre-existing valid partial; never claim filesystem publication can be rolled back by dropping a future.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## A03

**Interrupt and settle scratch queries**

Owner: `harness` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Implement context-aware scratch query execution with interrupt-then-join, reusing ScratchDb InterruptHandle and timeout settlement.

**Dependencies:** B00.

**Exclusive leases:** scratch. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/scratch/execute.rs` — existing
- `crates/saya-harness/src/scratch/mod.rs` — existing
- `crates/saya-harness/src/scratch/tools.rs` — existing
- `crates/saya-harness/tests/scratch.rs` — existing

**Red or verification target:** Cancel an active runaway scratch query through the public executor; a subsequent real scratch query succeeds, proving connection ownership was released after settlement.

**Evidence anchors:** `crates/saya-harness/src/scratch/execute.rs:40`; `crates/saya-harness/src/scratch/open.rs:34`; `crates/saya-harness/tests/scratch.rs:549`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## A04

**Cancel scratch imports transactionally**

Owner: `harness` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Check cancellation before parsing, around bounded insertion loops and before transaction commit; settle the blocking task before returning. Retain import timeout and row/byte limits.

**Dependencies:** A03.

**Exclusive leases:** scratch. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/scratch/import.rs` — existing
- `crates/saya-harness/src/scratch/tools.rs` — existing
- `crates/saya-harness/tests/scratch_import.rs` — existing

**Red or verification target:** Cancel a replacement import before commit: original table survives and next operation works. Cancel after actual commit: report the committed boundary truthfully. No detached blocking task.

**Evidence anchors:** `crates/saya-harness/src/scratch/import.rs:63`; `crates/saya-harness/src/scratch/import.rs:125`; `crates/saya-harness/src/scratch/import.rs:140`; `crates/saya-harness/src/scratch/import.rs:162`.

**Risk:** Parsing may take bounded time before observing cancellation; inspect duckdb API only if native interruption is added. Do not assume lack of a current token makes cooperative checks impossible.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## A05

**Wire cancellation through composite execution**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Forward ToolExecutionContext into fetch and scratch members. Preserve database/runner/host token behavior and late-effect suppression.

**Dependencies:** A02, A04.

**Exclusive leases:** tool-dispatch. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/runtime_tests.rs` — existing
- `crates/saya-cli/src/agent/tools/run_tools.rs` — existing

**Red or verification target:** Run a synthetic agent turn through the real composite, cancel during one fetch/import and assert no requested follow-up effect executes. This tests the wiring boundary, not the backend cases again.

**Evidence anchors:** `crates/saya-cli/src/agent/tools/run_tools.rs:185`; `crates/saya-cli/src/agent/runtime.rs:582`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## B00

**Freeze baseline and activate packet ownership**

Owner: `lead` · Kind: `gate` · Milestone class: `foundation` · State: `planned`.

**Outcome:** Refresh HEAD/status, active processes, actual crate graph, baseline gate evidence and chosen decisions. Expand candidate file sets, assign one writer and archive stale packets. Record exact baselines; no product edits.

**Dependencies:** None.

**Exclusive leases:** Read-only; no production write lease. Path overlaps also exclude concurrency.

**Owned footprint:**

No production/test write permission. Write scratch findings/evidence only in the assigned private orchestration area.

**Red or verification target:** N/A: activation and existing baseline validation, not a behavior change.

**Evidence anchors:** `docs/claude-code-second-review.md`; `Cargo.toml`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** orchestrate-saya, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## C01

**Resolve bounded instruction-source settings**

Owner: `config` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Resolve instruction enablement, personal source location and limits. Defaults follow DECISIONS; project config cannot alter enforcement authority or choose arbitrary personal files.

**Dependencies:** B00.

**Exclusive leases:** config-schema. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-config/src/instructions.rs` — NEW proposed
- `crates/saya-config/src/lib.rs` — existing
- `crates/saya-config/src/model.rs` — existing
- `crates/saya-config/src/resolve.rs` — existing
- `crates/saya-config/tests/instructions.rs` — NEW proposed

**Red or verification target:** Resolution precedence and malformed limits are observable through resolved config; untrusted project overrides cannot redirect personal-source reads.

**Evidence anchors:** `crates/saya-config/src/model.rs`; `crates/saya-config/src/layers.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-config, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-config --all-targets --locked -- -D warnings`
- `cargo test -p saya-config --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## C02

**Read bounded root and personal instruction sources**

Owner: `harness` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Load at most root AGENTS.md and the explicit personal file through bounded contained IO; return body, source identity/hash, and truncation metadata. No discovery walk/imports.

**Dependencies:** B00.

**Exclusive leases:** harness-exports, instructions. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/instructions.rs` — NEW proposed
- `crates/saya-harness/src/lib.rs` — existing
- `crates/saya-harness/tests/instructions.rs` — NEW proposed

**Red or verification target:** Public loader covers oversize UTF-8, missing optional file, symlink escape and stable hash/order. It never reads a sentinel outside allowed roots.

**Evidence anchors:** `crates/saya-harness/src/workspace/contain.rs`; `crates/saya-harness/src/workspace/patch.rs`.

**Risk:** NEW module; return a capability-owned record. No dependency on saya-config; caller supplies resolved settings.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## C03

**Project instruction context into provider requests**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Compose sources through existing ContextBlock/preflight, with deterministic order, bounded source refresh and invalidation on identity/privacy change. No system authority or extra provider request.

**Dependencies:** C01, C02.

**Exclusive leases:** context-runtime, cli-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/context_sources.rs` — NEW proposed
- `crates/saya-cli/src/agent/mod.rs` — existing
- `crates/saya-cli/src/agent/runtime.rs` — existing
- `crates/saya-cli/src/agent/runtime_preflight_tests.rs` — existing
- `crates/saya-cli/src/interactive/session_universe.rs` — existing

**Red or verification target:** Capture the actual provider request: instructions appear once with source labels and participate in admission; malicious instruction cannot execute a denied tool.

**Evidence anchors:** `crates/saya-cli/src/agent/runtime.rs:440`; `crates/saya-cli/src/agent/system_prompt.rs:181`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## C04

**Show loaded context sources**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Expose a /context view of loaded sources, limits/truncation, scope and omitted reasons using the actual projection metadata.

**Dependencies:** C03.

**Exclusive leases:** cli-surface. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing

**Red or verification target:** Real slash dispatch agrees with the request source receipt; a rejected source is shown as omitted without printing its content.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_continuation/summary.rs`; `crates/saya-cli/src/slash/registry.rs`.

**Risk:** NEW surface; inspect existing parser first and extend it without replacing established meanings.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D01

**Define versioned continuation records**

Owner: `contracts` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Define bounded source boundary/scope, generated narrative, user notes and deterministic file/check facts with independent provenance; no executable grants.

**Dependencies:** B00.

**Exclusive leases:** types-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-types/src/continuation.rs` — NEW proposed
- `crates/saya-types/src/lib.rs` — existing
- `crates/saya-types/tests/continuation.rs` — NEW proposed

**Red or verification target:** Public serialization/validation rejects unsupported versions, invalid counts/bounds and inconsistent provenance shapes; absence remains representable for legacy sessions. Do not claim that deserializing a hash authenticates an observed result.

**Evidence anchors:** `crates/saya-store/src/lib.rs:60`; `crates/saya-cli/src/interactive/session_state.rs:57`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-contracts, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-types --all-targets --locked -- -D warnings`
- `cargo test -p saya-types --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D02

**Persist continuation extensions without data loss**

Owner: `storage` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Add optional versioned continuation to the session envelope, redacting and bounding at the store boundary. Preserve old sessions and refuse destructive rewrites of unknown extensions.

**Dependencies:** D01.

**Exclusive leases:** session-store, store-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-store/src/continuation.rs` — NEW proposed
- `crates/saya-store/src/filesystem.rs` — existing
- `crates/saya-store/src/lib.rs` — existing
- `crates/saya-store/tests/sessions.rs` — existing

**Red or verification target:** Load legacy session, save/load safe continuation, reject an over-cap extension without changing previous bytes, and exercise unknown-version handling plus secret sentinel redaction.

**Evidence anchors:** `crates/saya-store/src/lib.rs:60`; `crates/saya-store/src/filesystem.rs:66`.

**Risk:** Downgrade write compatibility is not guaranteed; document backup/roll-forward behavior. Do not silently mutate migration state.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-storage, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-store --all-targets --locked -- -D warnings`
- `cargo test -p saya-store --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D03

**Build deterministic continuation facts**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Build a bounded record from actual session task/file/check observations and stable redacted-turn boundary. Generated prose cannot populate verified-result facts.

**Dependencies:** D01.

**Exclusive leases:** cli-exports, continuation-projector. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_continuation.rs` — existing
- `crates/saya-cli/src/interactive/session_continuation/record.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_continuation/record_tests.rs` — NEW proposed

**Red or verification target:** A forged summary does not become a passed test or file observation; actual operations produce revision-bound facts with caps and source boundary intact.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_state.rs:205`; `crates/saya-cli/src/interactive/session_continuation/summary.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D04

**Save validated compaction continuation**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Save a safe continuation when compaction completes, keeping transcript unchanged and separating model narrative from source facts. Save failure retains previous valid state.

**Dependencies:** D02, D03.

**Exclusive leases:** session-runtime, context-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/compact_task.rs` — existing
- `crates/saya-cli/src/interactive/session_compact.rs` — existing
- `crates/saya-cli/src/interactive/session_compact_bounds_tests.rs` — existing
- `crates/saya-cli/src/interactive/session_compact_tests.rs` — existing
- `crates/saya-cli/src/interactive/session_loop.rs` — existing
- `crates/saya-cli/src/interactive/session_state.rs` — existing

**Red or verification target:** Compaction produces persisted bounded continuation once; a sensitive narrative is omitted or refused, and failed save does not mark unsaved continuation durable.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_compact.rs:98`; `crates/saya-cli/src/interactive/session_state.rs:283`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D05

**Restore scoped continuation on resume**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Restore only validated continuation and matching source boundary, rechecking workspace/profile/privacy; label historical checks and refresh file digests. Never restore grants from notes.

**Dependencies:** D04, C03.

**Exclusive leases:** session-runtime, context-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/context_sources.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_continuation/summary.rs` — existing
- `crates/saya-cli/src/interactive/session_continuation_tests.rs` — existing
- `crates/saya-cli/src/interactive/session_resume.rs` — existing
- `crates/saya-cli/src/interactive/session_resume_tests.rs` — existing
- `crates/saya-cli/src/interactive/session_state.rs` — existing

**Red or verification target:** Compact/restart request contains permitted summary and correct tail; changed privacy/workspace/source prefix invalidates it; old sessions still resume; stale tests remain historical.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_resume.rs:45`; `crates/saya-cli/src/interactive/session_state.rs:224`.

**Risk:** Filtering changes message counts; use stable source identity, not compacted_turns after filtering.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D06

**Report session capacity before the save ceiling**

Owner: `storage` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Expose bounded serialized capacity using the same store accounting as saves; identify the 3 MiB warning and 4 MiB refusal without allocating an unbounded full record. No successor publication in this packet.

**Dependencies:** D02.

**Exclusive leases:** session-store. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-store/src/filesystem.rs` — existing
- `crates/saya-store/src/lib.rs` — existing
- `crates/saya-store/src/session_lifecycle.rs` — NEW proposed
- `crates/saya-store/tests/session_lifecycle.rs` — NEW proposed

**Red or verification target:** Existing session-store boundary distinguishes below-warning, warning and over-cap records with actual serialization; failed save preserves previous bytes. Do not mirror constants in a source-grep test.

**Evidence anchors:** `crates/saya-store/src/filesystem.rs:17`; `crates/saya-store/src/filesystem.rs:84`.

**Risk:** NEW lifecycle owner; split capacity status and successor transaction if fresh research finds independent ownership barriers.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-storage, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-store --all-targets --locked -- -D warnings`
- `cargo test -p saya-store --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D07

**Tombstone and clean session storage retryably**

Owner: `storage` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Implement validated-ID tombstone and cleanup mechanics for session JSON plus associated state/checkpoints, under the lifecycle owner lease precondition. Tombstones live outside the removable session tree. Independent preferences/knowledge remain. Runtime lock coordination is D12, not a new store-to-harness dependency.

**Dependencies:** D06.

**Exclusive leases:** session-store. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-store/src/filesystem.rs` — existing
- `crates/saya-store/src/session_lifecycle.rs` — NEW proposed
- `crates/saya-store/tests/session_lifecycle.rs` — NEW proposed

**Red or verification target:** Traversal/symlink cannot touch outside state; partial cleanup reports pending and retry converges; repeated tombstone cleanup is idempotent; independent knowledge survives. Active-session refusal is owned by D12.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_paths.rs:15`; `crates/saya-store/src/filesystem.rs:29`.

**Risk:** saya-store cannot depend on saya-harness::RunLock. Keep persistence mechanics here; expose no user-facing deletion until D12 coordinates the real lock. Never delete the lock then claim exclusivity persists.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-storage, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-store --all-targets --locked -- -D warnings`
- `cargo test -p saya-store --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D08

**Offer explicit session rotation**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Show near-capacity status and an explicit rotate action that creates/opens the successor with validated continuation, preserving parent access.

**Dependencies:** D05, D11.

**Exclusive leases:** cli-surface, session-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/session_loop.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing

**Red or verification target:** Real session reaches warning, rotates, and can reopen parent; cancellation of rotate leaves active session selected and no lost transcript.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_loop.rs`; `crates/saya-cli/src/interactive/session_resume.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D09

**Expose session deletion and retention inspection**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Provide explicit session delete and storage listing through shared operations with active-session refusal and clear independent-memory retention text; no automatic expiry.

**Dependencies:** D12.

**Exclusive leases:** cli-surface. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/cli.rs` — existing
- `crates/saya-cli/src/commands/mod.rs` — existing
- `crates/saya-cli/src/commands/sessions.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing
- `crates/saya-cli/tests/session_lifecycle_cli.rs` — NEW proposed

**Red or verification target:** Process and TUI dispatch remove only requested inactive session, report pending cleanup accurately, and never prompt in a scripted run.

**Evidence anchors:** `crates/saya-cli/src/cli.rs`; `crates/saya-cli/src/interactive/session_paths.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D10

**Inspect and edit durable session notes**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Expose bounded user note inspect/set/clear operations on the existing continuation record, separate from generated summary and database knowledge.

**Dependencies:** D05.

**Exclusive leases:** cli-surface, session-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/mod.rs` — existing
- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/session_notes.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_state.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing

**Red or verification target:** Note edit survives restart; clear removes note from disk/request; too-large/sensitive input preserves previous record; notes cannot assert authority.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_state.rs:283`; `crates/saya-store/src/lib.rs:60`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D11

**Publish an explicit successor session atomically**

Owner: `storage` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Publish bounded successor with parent linkage before changing selection, using a retry identity and preserving the parent transcript. No automatic rotation or grant transfer.

**Dependencies:** D02, D06.

**Exclusive leases:** session-store. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-store/src/filesystem.rs` — existing
- `crates/saya-store/src/lib.rs` — existing
- `crates/saya-store/src/session_lifecycle.rs` — NEW proposed
- `crates/saya-store/tests/sessions.rs` — existing

**Red or verification target:** Failure before/after successor publication keeps parent usable; repeated same rotation request yields one child; oversize seed is refused without modifying either saved record.

**Evidence anchors:** `crates/saya-store/src/filesystem.rs:17`; `crates/saya-store/src/filesystem.rs:84`.

**Risk:** NEW lifecycle owner; split capacity status and successor transaction if fresh research finds independent ownership barriers.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-storage, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-store --all-targets --locked -- -D warnings`
- `cargo test -p saya-store --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## D12

**Coordinate deletion with the live session lock**

Owner: `harness` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Use existing RunLock in the harness lifecycle owner before store tombstone/cleanup. New session acquisition checks stable tombstone before and after lock acquisition; refuse a live holder and prevent new same-ID acquisition during/after deletion. Preserve dependency direction; no store-to-harness edge.

**Dependencies:** D07.

**Exclusive leases:** session-runtime, harness-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_paths.rs` — existing
- `crates/saya-cli/src/interactive/session_runtime.rs` — existing
- `crates/saya-harness/src/lib.rs` — existing
- `crates/saya-harness/src/session_lifecycle.rs` — NEW proposed
- `crates/saya-harness/tests/session_lifecycle.rs` — NEW proposed

**Red or verification target:** Hold a real session lock in one process and attempt deletion in another: refusal and unchanged data. Interleave acquisition/deletion: only one succeeds, tombstone prevents resurrection, and no lock inode is removed while its exclusivity is still relied on.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_runtime.rs:12`; `crates/saya-cli/src/interactive/session_paths.rs:15`; `crates/saya-harness/src/lock.rs`.

**Risk:** Lock is currently inside the removable state directory. Document exact tombstone/lock ordering before code; a source-level PID check is not enough. Concurrent use by older binaries that ignore the tombstone is unsupported and must be stated.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E01

**Verify patch and append digests inside workspace commit**

Owner: `harness` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Add fresh-read digest guards to patch/append commits and serialize Saya writers using the existing anchored path; retain atomic publication. Whole-file guarded create/replace is E09.

**Dependencies:** B00.

**Exclusive leases:** workspace-commit, harness-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/workspace/mod.rs` — existing
- `crates/saya-harness/src/workspace/patch.rs` — existing
- `crates/saya-harness/tests/workspace_patch.rs` — existing

**Red or verification target:** Same-size content changed since observation refuses in the public patch API; matching digest succeeds; existing crash-before-publish preservation still holds on supported platform branches.

**Evidence anchors:** `crates/saya-harness/src/workspace/patch.rs:40`; `crates/saya-harness/src/workspace/patch.rs:122`.

**Risk:** Optimistic guards do not guarantee OS-level CAS against non-cooperating external writers. Extend platform branches without claiming untested platforms.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E02

**Require observed digest for agent edits and appends**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Require observed content digest for existing-file replace/append and call harness guards; offset zero is only unguarded create-if-absent. Clear reread/retry error updates schema/help.

**Dependencies:** E01.

**Exclusive leases:** tool-schema, workspace-adapters. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/tools/database_tools/definitions.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_append.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_args.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_replace.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_shared.rs` — existing
- `crates/saya-cli/src/agent/tools/workspace_edit_append_tests.rs` — existing
- `crates/saya-cli/src/agent/tools/workspace_edit_tests.rs` — existing

**Red or verification target:** Existing-file call without digest changes nothing; same-size stale digest refuses; valid digest changes intended bytes; append continuation remains possible after fresh read.

**Evidence anchors:** `crates/saya-cli/src/agent/tools/database_tools/definitions.rs:276`; `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_replace.rs:12`.

**Risk:** One edit family outcome with several small adapters. Split replace and append if packet exceeds 45 minutes; keep registry lease exclusive. Reuse typed error if semantically correct; any error contract change needs explicit agent-contract lease.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E03

**Close whole-file overwrite bypass**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Make whole-file tool creation explicitly create-if-absent and existing-file replacement digest-guarded through E01. Do not invent a second append API.

**Dependencies:** E02, E09.

**Exclusive leases:** tool-schema, workspace-adapters. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/tools/database_tools/definitions.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_write.rs` — existing
- `crates/saya-cli/src/agent/tools/workspace_write_tests.rs` — existing

**Red or verification target:** workspace_write cannot overwrite an existing file without current digest; concurrent create collision preserves original; bounded valid create/replace succeeds.

**Evidence anchors:** `crates/saya-cli/src/agent/tools/database_tools/workspace_write.rs:31`.

**Risk:** Behavior/API change: old model arguments receive a useful refusal, not an unsafe compatibility fallback.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E04

**Define checkpoint state and recovery policy**

Owner: `contracts` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Define prepared/committed/uncertain direct-edit metadata, digest identity, source kind, quota and restore eligibility. Scope is opt-in private source recovery per DECISIONS.

**Dependencies:** B00.

**Exclusive leases:** types-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-types/src/edit_checkpoint.rs` — NEW proposed
- `crates/saya-types/src/lib.rs` — existing
- `crates/saya-types/tests/edit_checkpoint.rs` — NEW proposed

**Red or verification target:** Serialization/validation preserves uncertain state, refuses impossible version/digest/size metadata, and cannot represent a subprocess rollback as a direct-file checkpoint.

**Evidence anchors:** `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_shared.rs`.

**Risk:** NEW persisted contract. Do not embed raw before-image bytes in general session JSON.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-contracts, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-types --all-targets --locked -- -D warnings`
- `cargo test -p saya-types --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E05

**Store bounded private checkpoint before-images**

Owner: `storage` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Persist byte-faithful admitted source snapshots and manifest transitions outside workspace, with private permissions, quotas, atomic publish and cleanup participation.

**Dependencies:** E04.

**Exclusive leases:** checkpoint-store, store-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-store/src/edit_checkpoints.rs` — NEW proposed
- `crates/saya-store/src/lib.rs` — existing
- `crates/saya-store/tests/edit_checkpoints.rs` — NEW proposed

**Red or verification target:** Raw admitted source round-trips exactly; known-secret/credential-path policy and quota refuse before capture; disk failure preserves last good manifest. Deletion of actual checkpoints is proven after D07/E08, not by a fake cleanup claim here.

**Evidence anchors:** `crates/saya-store/src/private_file.rs`; `crates/saya-store/src/replace.rs`; `crates/saya-cli/src/interactive/session_paths.rs`.

**Risk:** Raw source snapshots may contain sensitive source despite screening: opt-in and retention disclosure required; never describe as redacted file restore.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-storage, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-store --all-targets --locked -- -D warnings`
- `cargo test -p saya-store --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E06

**Capture checkpoints around direct file commits**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Route admitted direct edits through prepare snapshot → guarded commit → finalize manifest; recovery enabled only by explicit session choice and policy.

**Dependencies:** E03, E05.

**Exclusive leases:** workspace-adapters, session-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_append.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_replace.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_shared.rs` — existing
- `crates/saya-cli/src/agent/tools/database_tools/workspace_write.rs` — existing
- `crates/saya-cli/src/interactive/session_universe.rs` — existing

**Red or verification target:** Capture failure prevents recovery-required edit; crash after file commit yields reconcilable uncertain record; guard failure never marks committed. Subprocess effects produce no undo claim.

**Evidence anchors:** `crates/saya-cli/src/agent/tools/database_tools/workspace_edit_shared.rs:24`; `crates/saya-cli/src/agent/tools/database_tools/workspace_write.rs:64`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E07

**Restore only the last matching direct edit**

Owner: `harness` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Restore an admitted checkpoint through guarded workspace operations only if current post-edit digest/identity match; preserve original absent/existing state and dirty baseline.

**Dependencies:** E06.

**Exclusive leases:** checkpoint-restore, harness-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/workspace/mod.rs` — existing
- `crates/saya-harness/src/workspace/restore.rs` — NEW proposed
- `crates/saya-harness/tests/workspace_restore.rs` — NEW proposed

**Red or verification target:** Round-trip a direct edit including new file; human edit after Saya write causes refusal; unknown/uncertain manifest is reconciled or refused; no whole-tree reset.

**Evidence anchors:** `crates/saya-harness/src/workspace/patch.rs`; `crates/saya-harness/src/workspace/contain.rs`.

**Risk:** Delete-created-file restore needs a guarded no-follow unlink primitive; include it only for this outcome, not a generic deletion capability.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E08

**Review changes and explicitly restore a checkpoint**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Show bounded changed-file summaries/diffs and a user-directed restore operation with direct-edit coverage, conflicts and quota state visible.

**Dependencies:** E07, D09.

**Exclusive leases:** cli-surface. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/mod.rs` — existing
- `crates/saya-cli/src/interactive/session_changes.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing

**Red or verification target:** Real dispatch previews and restores one unchanged checkpoint; refuses changed current file; huge diff truncates visibly; command-created changes are labeled outside checkpoint coverage.

**Evidence anchors:** `crates/saya-cli/src/interactive/tui/dispatch/mod.rs`; `crates/saya-cli/src/slash/registry.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## E09

**Guard whole-file create and replacement**

Owner: `harness` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Provide create-if-absent and observed-digest replacement using the same fresh-read admission and publication boundary as patch. No optional bypass for agent overwrite callers.

**Dependencies:** E01.

**Exclusive leases:** workspace-commit, harness-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-harness/src/workspace/contain.rs` — existing
- `crates/saya-harness/src/workspace/guarded_write.rs` — NEW proposed
- `crates/saya-harness/src/workspace/mod.rs` — existing
- `crates/saya-harness/tests/workspace_patch.rs` — existing

**Red or verification target:** Create collision preserves existing content; missing/stale replacement digest refuses same-size changed content; valid replacement retains atomic-failure guarantees.

**Evidence anchors:** `crates/saya-harness/src/workspace/patch.rs:40`; `crates/saya-harness/src/workspace/patch.rs:122`.

**Risk:** Optimistic guards do not guarantee OS-level CAS against non-cooperating external writers. Extend platform branches without claiming untested platforms.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-harness, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-harness --all-targets --locked -- -D warnings`
- `cargo test -p saya-harness --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## H01

**Verify the existing scripted coding entry and permissions**

Owner: `cli` · Kind: `research` · Milestone class: `core` · State: `planned`.

**Outcome:** Known entry: saya --workspace <dir> --turn-file <file> already runs one turn and exits (session_loop.rs:259–292; tests/turn_file.rs). Freeze supported explicit grant/output/cancellation behavior through an isolated runtime probe before H02; do not repeat broad source discovery or add a session command.

**Dependencies:** B00.

**Exclusive leases:** Read-only; no production write lease. Path overlaps also exclude concurrency.

**Owned footprint:**

No production/test write permission. Write scratch findings/evidence only in the assigned private orchestration area.

**Red or verification target:** N/A: bounded research/decision packet. Identify a runnable no-DB edit/test scenario and the exact missing behavior before H02.

**Evidence anchors:** `crates/saya-cli/src/app.rs:195`; `crates/saya-cli/src/interactive/session_loop.rs`; `crates/saya-cli/src/cli.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** orchestrate-saya, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## H02

**Close only demonstrated turn-file coding gaps**

Owner: `cli` · Kind: `conditional-repair` · Milestone class: `core` · State: `planned`.

**Outcome:** Verify-first: exercise no-DB read/edit/test through existing --workspace/--turn-file with explicit existing grants. If it works, deliver evidence and documentation only; otherwise repair the smallest shared operation gap. No new command, alternate loop or ask permission widening.

**Dependencies:** H01.

**Exclusive leases:** cli-surface, session-runtime, context-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_loop.rs` — existing
- `crates/saya-cli/src/interactive/session_runtime.rs` — existing
- `crates/saya-cli/tests/turn_file.rs` — existing

**Red or verification target:** Extend tests/turn_file.rs for the missing no-DB coding composition scenario. A failure must reproduce a real gap before a source fix; a baseline pass means no production change is required.

**Evidence anchors:** `crates/saya-cli/src/app.rs:195`; `crates/saya-cli/src/agent/runtime.rs:582`.

**Risk:** If current --turn-file already satisfies entry semantics, H02 is narrowed to documented gap or marked verified/no-change with evidence, not rebuilt.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## H03

**Align scripted terminal outcomes and cancellation**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Ensure scripted coding reports one terminal result with documented exit, bounded machine output and cancellation/partial-work status using existing events.

**Dependencies:** H02, A05.

**Exclusive leases:** session-runtime, render. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/render/mod.rs` — existing
- `crates/saya-cli/src/stream_render.rs` — existing
- `crates/saya-cli/tests/turn_file.rs` — existing

**Red or verification target:** Cancel headless coding while active: one terminal outcome, no follow-up effect, useful retained partial result, and no mixed human text on machine-output stdout.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_emit.rs`; `crates/saya-cli/src/stream_render.rs`.

**Risk:** Do not promise new exit numbers until H01 records current public conventions.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## M01

**Define explicit worktree preference records**

Owner: `contracts` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Define bounded explicit-user preference records scoped to canonical worktree, with provenance and no execution authority; separate from typed database knowledge.

**Dependencies:** B00.

**Exclusive leases:** types-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-types/src/lib.rs` — existing
- `crates/saya-types/src/project_preference.rs` — NEW proposed
- `crates/saya-types/tests/project_preference.rs` — NEW proposed

**Red or verification target:** Public contract rejects invalid scope/bounds and generated-as-user provenance; stable IDs support update/delete without rebinding another workspace.

**Evidence anchors:** `crates/saya-types/src/lib.rs`; `crates/saya-store/src/knowledge_items/mod.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-contracts, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-types --all-targets --locked -- -D warnings`
- `cargo test -p saya-types --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## M02

**Persist scoped preferences in existing store**

Owner: `storage` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Add a separate bounded preference table using existing migration/transaction/redaction conventions; list/update/forget by exact worktree scope.

**Dependencies:** M01.

**Exclusive leases:** store-schema, store-exports. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-store/src/lib.rs` — existing
- `crates/saya-store/src/migration.rs` — existing
- `crates/saya-store/src/project_preferences.rs` — NEW proposed
- `crates/saya-store/src/sqlite.rs` — existing
- `crates/saya-store/tests/migration.rs` — existing
- `crates/saya-store/tests/project_preferences.rs` — NEW proposed

**Red or verification target:** Two worktrees remain isolated, quota/update transaction preserves prior value on failure, forgetting removes text from recall and follows declared physical cleanup behavior.

**Evidence anchors:** `crates/saya-store/src/sqlite.rs`; `crates/saya-store/src/knowledge_items/writes.rs`.

**Risk:** Existing PreferenceValue deliberately forbids free prose, and its table is one value per kind. Preserve that contract; a separate project-guidance record/table in the existing state DB reuses connection/security mechanisms without weakening it. Allocate next migration version from live HEAD, never assume 9.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-storage, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-store --all-targets --locked -- -D warnings`
- `cargo test -p saya-store --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## M03

**Recall project preferences through context projection**

Owner: `knowledge` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Recall only explicitly stored current-worktree preferences within aggregate budget, with source labels and invalidation after forget/scope change.

**Dependencies:** M02, C03.

**Exclusive leases:** context-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/agent/context_sources.rs` — NEW proposed
- `crates/saya-cli/src/agent/mod.rs` — existing
- `crates/saya-cli/src/agent/project_preferences.rs` — NEW proposed
- `crates/saya-cli/src/agent/runtime.rs` — existing

**Red or verification target:** Actual provider request contains allowed scope only and drops forgotten text next turn; forged preference requesting host access cannot change approval.

**Evidence anchors:** `crates/saya-cli/src/agent/recall_context/mod.rs:66`; `crates/saya-cli/src/agent/runtime.rs:440`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-knowledge, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## M04

**Expose explicit project preference operations**

Owner: `cli` · Kind: `implementation` · Milestone class: `core` · State: `planned`.

**Outcome:** Expose inspect/add/edit/forget as one bounded preference CRUD family through shared operations, clearly separate from existing database remember/contracts commands.

**Dependencies:** M02.

**Exclusive leases:** cli-surface. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/cli.rs` — existing
- `crates/saya-cli/src/commands/mod.rs` — existing
- `crates/saya-cli/src/commands/preferences.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing

**Red or verification target:** User operations round-trip and update next-turn recall; no DB profile required; no model extraction silently creates permanent preferences.

**Evidence anchors:** `crates/saya-cli/src/cli.rs`; `crates/saya-cli/src/slash/registry.rs`.

**Risk:** If the command family exceeds one short handoff, split read/forget from write/update before launch; same semantic owner.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## M05

**Ship reusable instruction workflow examples**

Owner: `release` · Kind: `documentation` · Milestone class: `follow-on` · State: `planned`.

**Outcome:** Provide small fix-test, query-review and dbt-review instruction examples using implemented native capabilities. No hook execution/plugin loader.

**Dependencies:** C04, M04.

**Exclusive leases:** docs. Path overlaps also exclude concurrency.

**Owned footprint:**

- `docs/coding-workflows.md` — NEW proposed
- `examples/coding/AGENTS.md` — NEW proposed

**Red or verification target:** N/A for prose; execute each documented command against an isolated synthetic fixture and verify supported syntax.

**Evidence anchors:** `docs/commands.md`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** release-saya, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## O01

**Track winning configuration origins**

Owner: `config` · Kind: `implementation` · Milestone class: `follow-on` · State: `planned`.

**Outcome:** Return source attribution for user-facing effective settings without retaining secret values or changing precedence.

**Dependencies:** C01.

**Exclusive leases:** config-schema. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-config/src/config_origin.rs` — NEW proposed
- `crates/saya-config/src/layers.rs` — existing
- `crates/saya-config/src/lib.rs` — existing
- `crates/saya-config/src/resolve.rs` — existing
- `crates/saya-config/tests/config_origin.rs` — NEW proposed

**Red or verification target:** Conflicting layers resolve identically to baseline but identify winning source; secret origin contains reference/label only, including ignored untrusted project override.

**Evidence anchors:** `crates/saya-config/src/layers.rs:92`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-config, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-config --all-targets --locked -- -D warnings`
- `cargo test -p saya-config --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## O02

**Explain effective configuration sources**

Owner: `cli` · Kind: `implementation` · Milestone class: `follow-on` · State: `planned`.

**Outcome:** Expose source explanation in a backward-compatible config view and doctor guidance so users know which setting/file to edit.

**Dependencies:** O01, U02.

**Exclusive leases:** cli-surface. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/cli.rs` — existing
- `crates/saya-cli/src/commands/config.rs` — existing
- `crates/saya-cli/src/config/doctor.rs` — existing
- `crates/saya-cli/tests/config_doctor.rs` — NEW proposed

**Red or verification target:** Isolated conflicting config prints winner and ignored-source reason without secrets; existing config show machine shape remains compatible unless a new explain flag is selected.

**Evidence anchors:** `crates/saya-cli/src/commands/config.rs`; `crates/saya-cli/src/config/doctor.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## O03

**Prepare atomic between-turn config reload**

Owner: `cli` · Kind: `implementation` · Milestone class: `follow-on` · State: `planned`.

**Outcome:** Build and validate a candidate resolved runtime, then swap only at an idle boundary. Changed authority-sensitive settings use explicit existing recomposition/consent semantics.

**Dependencies:** O02, H03.

**Exclusive leases:** session-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/mod.rs` — existing
- `crates/saya-cli/src/interactive/session_reload.rs` — NEW proposed
- `crates/saya-cli/src/interactive/session_runtime.rs` — existing
- `crates/saya-cli/src/interactive/session_universe.rs` — existing

**Red or verification target:** Invalid reload keeps old runtime and ability to execute; in-flight reload waits/refuses; changed privacy invalidates context before next request; no stale grant silently survives changed scope.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_universe.rs`; `crates/saya-cli/src/interactive/session_commands.rs`.

**Risk:** Conditional follow-on: if existing authority recomposition cannot support this narrowly, record blocked decision and retain restart guidance; do not weaken policy to ship reload.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## O04

**Expose explicit config reload**

Owner: `cli` · Kind: `implementation` · Milestone class: `follow-on` · State: `planned`.

**Outcome:** Expose one explicit reload action with accepted/rejected changes and next steps; reuse existing /provider and /model for simple switches.

**Dependencies:** O03.

**Exclusive leases:** cli-surface. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_commands.rs` — existing
- `crates/saya-cli/src/interactive/session_emit.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/chain_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/slash.rs` — existing
- `crates/saya-cli/src/slash/help.rs` — existing
- `crates/saya-cli/src/slash/registry.rs` — existing

**Red or verification target:** Real TUI/headless dispatch reloads valid config only between turns, reports unchanged state on error and never enters an interactive prompt in a pipe.

**Evidence anchors:** `crates/saya-cli/src/interactive/session_commands.rs:100`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## Q01

**Prove the complete native coding journey**

Owner: `quality` · Kind: `tests` · Milestone class: `acceptance` · State: `planned`.

**Outcome:** Own one synthetic project journey crossing setup/instructions/edit/check/test/compact/restart surfaces, with deterministic provider fixtures; measure observable outcomes.

**Dependencies:** U05, U06, C04, D08, D10, E08, M04, H03, U02, U07, M03.

**Exclusive leases:** Read-only; no production write lease. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/tests/native_coding_journey.rs` — NEW proposed

**Red or verification target:** The full new user journey fails before relevant features exist; missing real tool composition, lost continuation or stale file handling fails despite mocked model responses.

**Evidence anchors:** `crates/saya-cli/tests/tui_pty_smoke.rs`.

**Risk:** Distinct cross-surface lifecycle risk only; do not repeat each backend unit case. Fake model proves plumbing, not model coding quality.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** test-saya, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## Q02

**Prove mixed code and database privacy boundaries**

Owner: `quality` · Kind: `tests` · Milestone class: `acceptance` · State: `planned`.

**Outcome:** Own one synthetic mixed task that inspects schema, edits local SQL/code and performs bounded DB validation, including malicious retrieved instructions and scope switching.

**Dependencies:** A05, C04, D05, E03, M04, H03.

**Exclusive leases:** Read-only; no production write lease. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/tests/mixed_coding_database.rs` — NEW proposed

**Red or verification target:** Attempted DB write and forged memory authority execute no side effect; protected row/secret sentinels never reach forbidden persistence/output while local coding still works.

**Evidence anchors:** `crates/saya-connectors/tests/read_only_contract.rs`; `crates/saya-cli/src/agent/recall_context/mod.rs`.

**Risk:** Reuse connector safety tests; this case owns composition/privacy boundaries, not another SQL parser matrix.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** test-saya, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## Q03

**Independently audit integrated correctness and authority**

Owner: `quality` · Kind: `review` · Milestone class: `acceptance` · State: `planned`.

**Outcome:** Read-only GLM review of the exact baseline-to-integration diff and real call paths; lead independently resolves findings through new bounded repair packets.

**Dependencies:** Q01, Q02. Full scope also requires O04, M05.

**Exclusive leases:** Read-only; no production write lease. Path overlaps also exclude concurrency.

**Owned footprint:**

No production/test write permission. Write scratch findings/evidence only in the assigned private orchestration area.

**Red or verification target:** N/A: independent source review. Every finding cites trigger, owner path and consequence; claims must survive runtime-composition checks.

**Evidence anchors:** `docs/claude-code-second-review.md`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** orchestrate-saya, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## Q04

**Publish accurate local capability and migration documentation**

Owner: `release` · Kind: `documentation` · Milestone class: `acceptance` · State: `planned`.

**Outcome:** Document implemented surfaces, source/continuation scope, limits, snapshot privacy, restore coverage, downgrade behavior and platform gaps; update tracked architecture ADR.

**Dependencies:** Q03.

**Exclusive leases:** docs. Path overlaps also exclude concurrency.

**Owned footprint:**

- `CHANGELOG.md` — existing
- `README.md` — existing
- `docs/adr-native-coding.md` — NEW proposed
- `docs/coding-workflows.md` — NEW proposed
- `docs/commands.md` — existing
- `docs/configuration.md` — existing
- `docs/memory.md` — existing

**Red or verification target:** N/A for prose. Execute documented commands; inspect data promises against the real binary. Include only accepted capabilities.

**Evidence anchors:** `docs/adr-0001-release-architecture.md`; `docs/commands.md`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** release-saya, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## Q05

**Run final local integration gates**

Owner: `lead` · Kind: `gate` · Milestone class: `acceptance` · State: `planned`.

**Outcome:** Freeze exact HEAD and run fmt, strict all-target workspace clippy, locked workspace tests, doctests and dependency audit when applicable. Archive counts/skips and exact commit.

**Dependencies:** Q04.

**Exclusive leases:** integration-build. Path overlaps also exclude concurrency.

**Owned footprint:**

No production/test write permission. Write scratch findings/evidence only in the assigned private orchestration area.

**Red or verification target:** N/A: acceptance gate. Zero matched tests never count; any failure becomes a scoped repair followed by affected gates.

**Evidence anchors:** `/Users/subodhsharma/Projects/saya-cli/AGENTS.md`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** orchestrate-saya, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## Q06

**Audit real native release artifacts and retained state exclusion**

Owner: `release` · Kind: `gate` · Milestone class: `acceptance` · State: `planned`.

**Outcome:** Build/package locally and run existing content scanners on actual native binary/archive/crate payloads. Verify no new session/preferences/checkpoint artifacts enter packages.

**Dependencies:** Q05.

**Exclusive leases:** integration-build, artifacts. Path overlaps also exclude concurrency.

**Owned footprint:**

No production/test write permission. Write scratch findings/evidence only in the assigned private orchestration area.

**Red or verification target:** N/A: actual artifact audit plus existing sentinel cases. Build provenance/platform signing remain separate and unverified unless separately run.

**Evidence anchors:** `scripts/package.sh`; `scripts/check-artifact-content.py`; `scripts/publish-crates.sh`.

**Risk:** Do not invoke a publisher to test packaging. No remote publish, tag or signing promise; report platforms not exercised.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** orchestrate-saya, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- Use the packet evidence requirement and VALIDATION.md; no fictitious red/green result for prose, research or gates.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U01

**Compute task-specific configuration readiness**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Produce query, AI and coding readiness from resolved config without network calls, preserving the legacy default doctor exit.

**Dependencies:** B00.

**Exclusive leases:** doctor. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/config/doctor.rs` — existing

**Red or verification target:** Table cases: valid provider/no DB, valid SQLite/missing AI key, no workspace, unresolved selected DB secret. Assert each task result, not renderer wording.

**Evidence anchors:** `crates/saya-cli/src/config/doctor.rs:11`; `crates/saya-cli/src/config/doctor.rs:108`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U02

**Expose task-aware doctor command**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Add doctor --task query|ai|coding and additive structured output. Unspecified task keeps existing exit behavior.

**Dependencies:** U01.

**Exclusive leases:** cli-surface, doctor. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/cli.rs` — existing
- `crates/saya-cli/src/commands/config.rs` — existing
- `crates/saya-cli/tests/config_doctor.rs` — NEW proposed

**Red or verification target:** Process-level invocation in isolated config homes returns the selected task exit and distinguishes configured from network-verified. Existing scripts without --task retain exit semantics.

**Evidence anchors:** `crates/saya-cli/src/cli.rs`; `crates/saya-cli/src/commands/config.rs`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U03

**Show coding readiness without a database**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Render independent AI/workspace/database status and visible project, database, data-file and demo entry choices. Surface existing /provider and /model commands.

**Dependencies:** U01.

**Exclusive leases:** tui-empty. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/tui/ui/surface/empty_state_tests.rs` — existing
- `crates/saya-cli/src/interactive/tui/ui/surface/splash.rs` — existing

**Red or verification target:** Render provider-only workspace setup: it is usable for coding and does not instruct the user to configure a required DB. Extend the existing empty-state snapshot with a distinct no-AI SQL case.

**Evidence anchors:** `crates/saya-cli/src/interactive/tui/ui/surface/splash.rs:10`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U04

**Reuse guided setup for selected startup intent**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Add an application entry accepting project/database/demo/data intent; project setup can skip the DB while keeping existing preview/confirmation and failure recovery.

**Dependencies:** B00.

**Exclusive leases:** setup. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/setup/flow.rs` — existing
- `crates/saya-cli/src/setup/mod.rs` — existing
- `crates/saya-cli/src/setup/prompt_database.rs` — existing
- `crates/saya-cli/src/setup/prompt_provider.rs` — existing
- `crates/saya-cli/tests/setup_cli.rs` — existing

**Red or verification target:** Injected setup IO selecting project configures only the provider, cancellation writes nothing, and a failed probe never silently commits. Extend existing flow tests.

**Evidence anchors:** `crates/saya-cli/src/setup/flow.rs:71`; `crates/saya-cli/src/setup/prompt_database.rs:40`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U05

**Hand terminal ownership to guided setup safely**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Implement one terminal handoff boundary for invoking the existing guided setup and returning to the idle session; always restore terminal ownership after success/error/cancellation. Do not wire new startup menu actions yet.

**Dependencies:** U04.

**Exclusive leases:** cli-surface, session-runtime. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_loop.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/interactive/tui/mod.rs` — existing
- `crates/saya-cli/tests/tui_pty_smoke.rs` — existing

**Red or verification target:** Real PTY enters and exits existing guided setup through the handoff, including cancellation/failure: no nested runtime panic, lost input or double raw-mode owner.

**Evidence anchors:** `crates/saya-cli/src/setup/flow.rs:90`; `crates/saya-cli/src/interactive/tui/mod.rs`.

**Risk:** If terminal handoff exceeds one behavior, split terminal suspension from setup dispatch before launch; do not implement alternate setup logic.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U06

**Clarify endpoint-bound credential reference selection**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Verify-first: retain the existing suggested env name and presence indicator. Improve only confusing endpoint/reference labels if demonstrated; no new confirmation step, secret enumeration or implicit fallback. If already sufficient, deliver verified/no-change evidence.

**Dependencies:** U04.

**Exclusive leases:** setup. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/setup/prompt_provider.rs` — existing
- `crates/saya-cli/src/setup/setup_tests.rs` — existing

**Red or verification target:** Extend the existing setup flow only for a demonstrated regression: selected provider reference remains bound to its endpoint; compatible endpoint cannot inherit a different key. If current behavior passes, no production change and no invented red result.

**Evidence anchors:** `crates/saya-cli/src/setup/prompt_provider.rs:50`.

**Risk:** Retain existing bounds, privacy and authority; no unrelated cleanup.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.

## U07

**Wire startup choices to shared setup operations**

Owner: `cli` · Kind: `implementation` · Milestone class: `release` · State: `planned`.

**Outcome:** Wire the project/database startup choice through safe setup handoff, then recompose an idle session. Data-file/demo choices reuse existing entry operations or explicit commands without a duplicate wizard.

**Dependencies:** U03, U05.

**Exclusive leases:** cli-surface, session-runtime, tui-empty. Path overlaps also exclude concurrency.

**Owned footprint:**

- `crates/saya-cli/src/interactive/session_loop.rs` — existing
- `crates/saya-cli/src/interactive/tui/dispatch/session_actions.rs` — existing
- `crates/saya-cli/src/interactive/tui/ui/surface/splash.rs` — existing
- `crates/saya-cli/tests/tui_pty_smoke.rs` — existing

**Red or verification target:** Choose project setup from the real first screen and return ready to code with no DB. Cancel leaves prior config/session intact; noninteractive never enters the menu.

**Evidence anchors:** `crates/saya-cli/src/setup/flow.rs:90`; `crates/saya-cli/src/interactive/tui/mod.rs`.

**Risk:** If terminal handoff exceeds one behavior, split terminal suspension from setup dispatch before launch; do not implement alternate setup logic.

**Rollback:** Revert this packet before dependents; after consumers land, roll forward or revert the dependency closure. Preserve user files and persisted data.

**Required skills:** build-saya-cli, ponytail, test-audit. Use TDD for behavior changes; use existing owner tests before adding new files.

**Validation:**

- `git diff --check`
- `cargo fmt --check`
- `cargo clippy -p saya-cli --all-targets --locked -- -D warnings`
- `cargo test -p saya-cli --locked`
- Lead: real dispatch/PTY scenario where user behavior changes; independent scoped rerun; after local merge cargo check --workspace --all-targets --locked.

**Size and scope:** 20–45 active minutes target; split if more than one independent behavior or gate failure remains. No authority expansion, new crate/framework, unrelated refactor, broad test pruning, remote action, or automatic model memory. Respect packet scope and DECISIONS.md.
