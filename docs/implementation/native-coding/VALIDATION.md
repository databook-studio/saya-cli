# Native coding validation and evidence

No tests were run as part of writing this plan. The commands and scenarios below are execution requirements, not passed results. Use the repository's test-audit skill on every new, changed or reviewed test; this plan does not authorize a broad test-pruning campaign.

## One primary test owner per contract

| Contract | Primary owner boundary | Existing seam to extend | Why another boundary may be needed |
| --- | --- | --- | --- |
| Fetch/download cancellation and partial publication | saya-harness transport/workspace API | fetch adapter/download suites | A05 separately proves CLI forwards the actual turn token |
| Scratch interruption/rollback/connection release | saya-harness real DuckDB scratch API | tests/scratch.rs; tests/scratch_import.rs | A05 proves composition, not another scratch timeout matrix |
| Task readiness and exit compatibility | CLI configuration report and command process | doctor.rs inline tests; existing config/CLI suites | PTY owns first-screen behavior and terminal lifecycle |
| Instruction containment and byte caps | Harness loader | NEW focused loader contract suite using real temp files | C03 verifies actual request placement/admission and denial authority |
| Continuation serialization/migration/size | saya-store session API | tests/sessions.rs | D05 verifies privacy-aware request reconstruction across restart |
| Rotation/delete failure atomicity | Store lifecycle API | sessions suite; new focused lifecycle section if necessary | D08/D09 exercise real user command/active-session behavior |
| Stale edit guard | Harness fresh-read commit API | tests/workspace_patch.rs | Tool tests prove every exposed write-shaped adapter supplies the guard |
| Direct-edit restore | Harness recovery API plus private store | NEW recovery/store suites | E08 verifies scope/coverage shown by real UI, not a second byte-copy suite |
| Explicit project guidance | Store scope/admission boundary | NEW guidance suite, existing migration helpers | M03 proves actual request isolation and immediate forget invalidation |
| Scripted coding | Real saya process | tests/turn_file.rs | Q01 owns the larger setup/edit/compact/restart journey, not the same flag cases |

Before adding a test, name (1) its observable contract, (2) the credible regression, (3) why existing coverage would miss it, and (4) whether any proposed testing seam has a real production caller. Reject source inventories, copied constants, assertion-free smoke calls, tests whose mock implements the desired policy, and duplicate proofs. A parser/serialization/public-wire test can be valuable; a private helper call-shape test usually is not.

## Red and green evidence

For a behavior change, run the failing owner-boundary case before implementing. A missing public API compile failure can be red evidence when it is the actual proposed contract. A test that matches zero cases is neither red nor green. For a verify-first packet where baseline behavior already passes, report verified-no-change and avoid a fabricated failure or unnecessary production patch.

Representative scoped commands, with all touched crate packages included:

```bash
cargo fmt --check
cargo clippy -p saya-harness --all-targets --locked -- -D warnings
cargo test -p saya-harness --locked
```

For a concrete selected test, list it first and confirm it is exercised; record the actual command/filter and count. If a packet touches both CLI and harness, validate both. For new public examples/doc contracts, run corresponding doctests. Use `git diff --check` and inspect changed snapshots manually. Re-run only the affected scopes after new edits; do not burn time repeatedly running an unchanged green suite.

The lead runs independent scoped verification on immutable code before integration. After **every** local merge:

```bash
cargo check --workspace --all-targets --locked
```

No edits occur in that checkout until the command settles. Up to two other Cargo processes may use separate immutable worker checkouts if the measured resource budget allows it; the integration build consumes one build slot too.

## Combined user journeys

1. **Cold start to useful coding:** isolated empty config/state; no DB; configure provider through setup, load AGENTS.md, read/edit one file, run one permitted check, show changed files. Real process/PTY and fake provider responses exercise operations, not fake tool effects. Track time, number of setup actions, prompts and errors; label model output as a fixture.
2. **SQL without AI:** demo/schema/bounded query remains useful without provider authentication. Existing connector contracts own read-only enforcement; this journey owns the application route.
3. **Mixed coding and database task:** synthetic schema informs a source-file SQL fix, local check runs, and bounded query validates it. A write attempt is refused, protected row/credential sentinels do not cross prohibited sinks, and malicious retrieved instructions cannot widen authority.
4. **Next-day continuation:** compact, restart, change a file externally, resume. The goal/notes survive; source checks become historical, the stale digest refuses mutation, privacy changes invalidate unsafe context, and no grant is reconstructed from narrative.
5. **Edit recovery:** start with a dirty source file, make an admitted direct edit, conditionally restore it. The original dirty bytes survive. A later human edit causes restore refusal; arbitrary command edits are not represented as covered. Quota/disk failure cannot yield a false checkpoint success.
6. **Cancellation:** independently cancel query, process, fetch, download and scratch import. Record the actual settlement point and side effects. For import/transfer, test the commit boundary; no subsequent tool starts after cancellation wins. Do not assert a universal server-stop latency based on a UI event.
7. **Lifecycle and privacy:** hit capacity warning, rotate explicitly, recover an interrupted rotation, refuse active-session deletion, retry partial cleanup, and verify independent project guidance/database knowledge remain unless separately forgotten.
8. **Scripted entry:** existing `saya --workspace <dir> --turn-file <file>` with explicit supported grants completes a no-DB task and exits once. Missing authority refuses without hanging. Machine output stays parseable and prompt-free. `ask` keeps its existing semantics.

Q01/Q02 add only missing cross-boundary proof. Each implementation packet already owns its narrower red/green evidence. Reuse existing fixtures and exercise actual files, processes, SQLite/DuckDB and request capture where those are the claimed effects.

## Persistence and failure cases

- Load pre-extension session fixtures unchanged; unsupported extension versions cannot be overwritten silently.
- Validate source boundaries against the persisted redacted transcript, before privacy filtering changes message counts. Hash consistency does not authenticate model claims.
- Failed extension save preserves last-good session. Scope and privacy changes omit sensitive continuation with an explicit reason.
- Rotation has an idempotent request identity: crash before successor, after successor, before selection and after selection. Preserve parent and prevent duplicate children on retry.
- Deletion validates identifiers/containment, acquires the real session lock, tombstones before irreversible cleanup, and reports pending work. Test non-cooperating path replacement at the established no-follow boundary where supported.
- Snapshot prepare/write/finalize crashes yield explicit pending/uncertain states. Recovery never repeats an arbitrary tool effect to make the log look complete.
- New project guidance migration upgrades an existing state DB and leaves old typed preferences/knowledge intact. Choose the next migration version from current integration state, not this document's baseline.
- Credential-shaped snapshot paths and known-secret content are refused. This does not prove every possible secret is detected; opt-in source retention remains an explicit product property.

## Release milestone and final gates

Run on the exact chosen immutable integration HEAD:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo nextest run --workspace --locked
cargo test --workspace --doc --locked
```

Use `cargo test --workspace --locked` if nextest is unavailable and record the substitution. Run `cargo audit --deny warnings` when dependency changes occur. No dependency changes are planned; any proposed one receives a separate reviewed packet.

Run the existing artifact-content scanner on the actual native binary, archive and crate payloads using their current supported invocation. Run scanner sentinel tests if its policy changes. Ensure newly introduced private continuation, preference and checkpoint data is excluded. Do not call a publish script that publishes merely to obtain a package. No push, tag, release upload or remote CI trigger is authorized by this plan.

Independently review the final diff after all relevant behavior and fixes land. A repair invalidates the affected earlier evidence and may require a full re-gate at the final HEAD. Release-only/core/full acceptance records are separate; record the exact selected packet set.

## Handoff evidence record

Each accepted packet records:

- Packet ID, worker/session identity, branch, base and final commit, producer commits.
- Exact edited files and any approved scope expansion; production versus test/support diff size.
- SPEC REVIEW objections and lead resolution.
- Red command/result (or valid no-change/research rationale), green commands/counts/skips.
- Independent lead check, real CLI/PTY scenario where applicable, merged all-target result.
- Bounds, privacy, permission and migration facts; known platform/live limits.
- Local integration commit, downstream packets released, retained state/rollback instructions.

Keep scratch logs, worker packets, model output and credentials out of product commits. Archive evidence privately under the orchestration area. Reports with claimed success but no executed operations are not acceptance.

## Evidence levels

Source inspection establishes wiring/design. Deterministic fake-provider tests establish application behavior under specified model output. Local native process and database tests establish those host behaviors. Env-gated live-provider/database tests and platform probes establish only what actually ran. Human usability and coding-quality evaluation are separate and remain unmeasured until performed. Never turn one level into a broader capability claim.
