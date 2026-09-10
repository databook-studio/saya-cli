# ADR 0003: The scratch database — the one writable SQL surface

- Status: accepted. Reviewer sign-off on the narrowed hardening (decision 4) is recorded
  in this ADR before `scratch_sql` merges — [plan](../plan/PLAN.md) §9, M4-0.
- Date: 2026-09-10
- Supersedes: nothing. Complements [ADR 0001](adr-0001-release-architecture.md) and
  [ADR 0002](adr-0002-memory-and-contract-trust-model.md).
- Records why runs get a writable scratch database, why it deliberately does not travel
  the `DatabaseConnector` path, and what that separation costs.

## Context

The design's non-goal is absolute: **no write to a user-registered database, ever, not
just in v1** ([DESIGN](../plan/DESIGN.md) §2). The scratch database is named there as
the single exception, and the exception is not user data — it is run-scoped working
state.

Runs need it. An analysis run stages intermediate results, joins predictions against
gold answers, scores per-table findings, and re-queries its own working set across
steps ([DESIGN](../plan/DESIGN.md) §6.5). Doing that in workspace files means
hand-rolled CSV diffing inside the model's context; doing it in a user database is
forbidden. A relational engine is exactly the right tool for join-and-score, and DuckDB
is already in the dependency tree (`duckdb` 1.10505.0, [`Cargo.lock`](../Cargo.lock)
lines 981–982).

This is the first writable SQL in the product. Everything else in saya — every backend
the user points it at — crosses the read-only AST gate in
[`safety/read_only.rs`](../crates/saya-connectors/src/safety/read_only.rs). The design
therefore requires that scratch ship with its own ADR and its full test battery
([DESIGN](../plan/DESIGN.md) §6.5).

## Decision

### 1. What the scratch database is for

One DuckDB file per run, at `runs/<id>/scratch.duckdb`, opened by the run engine via
the raw `duckdb` crate — the seam the connector test fixtures already use
([`mysql_duckdb_contract.rs`](../crates/saya-connectors/tests/mysql_duckdb_contract.rs),
lines 59–62) — as staging, join, and scoring space for that run. It holds intermediate
results the run creates: staged extracts, joined predictions-vs-gold tables, scored
findings. It never holds anything the user registered, and it never serves `saya ask`
or any interactive session: the tool that drives it exists only in the run engine's
toolset ([plan](../plan/PLAN.md) §9, M4-2).

### 2. It must not travel the `DatabaseConnector` path

The read-only gate is unconditional today, and that is its entire value:

- Every `prepare_*` function takes exactly `(sql, max_rows)` — no mode, no permit, no
  exception parameter ([`safety/read_only.rs`](../crates/saya-connectors/src/safety/read_only.rs)
  lines 39–100).
- `prepare()` parses, enforces a single statement, walks the AST with a per-backend
  denylist guard, and rejects everything that is not a `Query`, an `Explain` wrapping
  one, or a `Show*` variant (`prepare` at lines 102–134; `allowed` at lines 179–199;
  CTE-wrapped writes rejected in `set_allowed`, lines 223–240).
- Independent oracles pin it: a property suite proves every accepted statement belongs
  to a read-only statement class
  ([`read_only_property.rs`](../crates/saya-connectors/tests/read_only_property.rs)),
  and the contract suite rejects the escape shapes — `ATTACH`, `COPY … TO`,
  `INSTALL httpfs`, `PRAGMA enable_external_access`, writes hidden in CTEs
  ([`read_only_contract.rs`](../crates/saya-connectors/tests/read_only_contract.rs)).
  Those suites are the merge condition for every phase of the plan
  ([plan](../plan/PLAN.md) §3) and are never modified.

A scratch database behind [`DatabaseConnector`](../crates/saya-connectors/src/lib.rs)
(lines 35–43) would need exactly one new thing: a "writes permitted" parameter on the
gate. The moment that parameter exists, the guarantee for all seven backends changes
shape — from *read-only by construction* to *read-only unless a flag was set wrong* —
and every call site becomes an audit target. A tool-level permit flag on the existing
connector is the same change wearing a narrower name.

The temptation is real because the connector already carries a `read_only` flag:
[`DuckDbConnector::open`](../crates/saya-connectors/src/duckdb/client.rs) accepts one
and sets DuckDB's `AccessMode` accordingly (lines 20–30). But access mode governs the
file lock, not what may flow: every statement still crosses `prepare_duckdb_sql`
before execution ([`duckdb/execute.rs`](../crates/saya-connectors/src/duckdb/execute.rs)
line 20), and the engine-level hardening — `enable_external_access(false)`, extension
and secret controls, `lock_configuration` — is applied and locked in
(`client.rs` lines 43–52) with a test proving even `SET enable_external_access = true`
fails at runtime after an AST-level bypass attempt (lines 89–109). Today no SQL write
can reach any user database through any path. That is the property to protect, and the
only way to protect it while adding a writer is to make the writer a different type
with a different policy — one that does not implement `DatabaseConnector` and never
enters the [`ConnectionRegistry`](../crates/saya-cli/src/connection/registry.rs)
(lines 29–33), so no existing SQL tool can select it as a connection and no
type-level path exists from a scratch write to a user database
([DESIGN](../plan/DESIGN.md) §6.5).

The cost is accepted and deliberate: a second validator pass (`sqlparser` again, its
own policy), a second open/timeout/interrupt plumbing, and a structural test asserting
the separation — duplicated plumbing bought to keep one invariant absolute. The
engine-level discipline is reused, not reworked: the same timeout, interrupt, and
await-on-cancel shape as the connector
([`execute.rs`](../crates/saya-connectors/src/duckdb/execute.rs) lines 90–99), and the
same row and byte caps on what reaches the model (lines 33–56).
### 3. Where it lives, who may write, and when it dies

**Lives:** inside the run directory the engine already creates — `runs/<id>/`
with `workspace/` and `state/`, set to 0700 on every create and resume
([`run_dir.rs`](../crates/saya-harness/src/run_dir.rs) lines 27–40 and 63–68), under a
runs root resolved from `SAYA_RUNS_DIR`, then the platform data home
([`paths.rs`](../crates/saya-harness/src/paths.rs) lines 15–29). The run's
single-writer lock (`run_dir.rs` lines 57–60) covers it.

**Who may write:** only the run's own episode, only through the `scratch_sql` tool,
only after the run's plan is approved with the `scratch` scope — runs approve scopes
once, at plan approval, and headless runs pre-declare them or refuse by construction
([DESIGN](../plan/DESIGN.md) §7). The tool declares an honest write-shaped effect; the
effect machinery already distinguishes write-shaped local state and the loop refuses
it unless permitted ([`tool.rs`](../crates/saya-agent/src/protocol/contracts/tool.rs)
lines 25–30), and read-only approval denies anything not read-shaped
([`approval.rs`](../crates/saya-agent/src/protocol/contracts/approval.rs) lines 19–25).

**When it dies:** with the run. The scratch file's lifetime is the run directory's
lifetime — created by the run, addressed only by the run's tools, removed when the run
directory is. Nothing promotes it to a shared asset; a fresh run starts from an empty
scratch. If run-directory retention or deletion is built later, it operates on run
directories as a whole — scratch never acquires an independent lifecycle.

### 4. The narrowed hardening — external access, in-run-dir paths only

The connector's DuckDB engine hardening
([`duckdb/client.rs`](../crates/saya-connectors/src/duckdb/client.rs) lines 43–52) is
carried onto the scratch connection but **narrowed, not disabled**:
`enable_external_access(true)` is permitted so corpus files can load, while
`enable_autoload_extension(false)`, community-extension and persistent-secret denial,
and `lock_configuration` stay exactly as the connector sets them. File reads
(`read_csv`, `ATTACH`) are admitted only for canonicalised paths inside the run dir —
validated by scratch's own parser pass, the containment seam the workspace already
uses: canonicalise-then-prefix, symlink refusal
([`workspace/contain.rs`](../crates/saya-harness/src/workspace/contain.rs) lines 58–86).
Objects outside the run dir are denied. The connector's function-deny posture is
re-evaluated for scratch here: the denylist
([`read_only_policy.rs`](../crates/saya-connectors/src/safety/read_only_policy.rs)
lines 35–45) stays in force on the scratch path except for the file-read carve-out
above, and the carve-out is enforced twice — by the validator's parse, and by the
property battery proving every accepted statement touches only the run dir
(fs-monitor plus a planted sentinel outside the run dir that stays unread).

**Fallback if review rejects even this:** files-only staging — no DuckDB file reads at
all; loads happen outside the engine. The design degrades cleanly
([DESIGN](../plan/DESIGN.md) §6.5); the scratch join/score capability survives, only
its ingestion path narrows.

### 5. DuckDB's ReadWrite semantics are pinned before the capability is advertised

DuckDB's ReadWrite file-creation behaviour is unverified and unpinned by any current
test — open question U3 ([DESIGN](../plan/DESIGN.md) §12;
[plan](../plan/PLAN.md) §2). M4-1 writes the semantics suite
(`crates/saya-harness/tests/scratch_semantics.rs`) *before* `scratch_sql` exists: what
ReadWrite creation does, what the locked configuration actually permits, `read_csv` on
a local file, and the absence of network functions without httpfs — on the bundled
1.10505.0. This ADR is amended with what the pinning reveals, and that amendment is
the entry condition for M4-2. If DuckDB's reality contradicts this design, the tests
stay red and the capability is not advertised — that is what pinning first means.

## Consequences

**Accepted costs.**

- A second SQL policy and validator exist alongside the connector's. Two places now
  reason about what SQL means; they drift only if a change lands in one and not the
  other, which the two-directional property battery exists to catch.
- Scratch cannot reuse connector plumbing. Open, timeout, interrupt, and caps are
  re-implemented in `saya-harness` following the connector's patterns rather than
  shared with it. Deliberate: shared plumbing would be the thin end of the conditional
  gate.
- The write capability is plan-gated, not per-call approved. In runs the user approves
  the scratch scope once with the plan; per-call prompts stay where they are for
  interactive SQL ([DESIGN](../plan/DESIGN.md) §7). An unattended run with scratch
  approved can write to its own scratch file without further prompts — bounded by the
  run dir, the budget, and the battery.
- Admission of `scratch_sql` is hidden until the scope is approved, the same
  hidden-not-advertised pattern the run tools use.

**Rejected alternatives.**

- *Reuse the existing connector with a permit flag.* Requires a writes-permitted
  parameter on the unconditional gate — the one change that turns "read-only by
  construction" into "read-only unless configured". Every backend call site becomes a
  place the guarantee can leak, and the read-only suites would grow conditional
  branches instead of staying byte-identical as the merge condition requires.
- *A long-lived scratch database shared between runs.* One run's intermediate state
  silently steering another run's query is cross-run bleed with no provenance, and it
  breaks run reproducibility — a run's inputs are its spec, not leftovers from a
  previous run ([plan](../plan/PLAN.md) §2, G3). It also needs a deletion story no one
  owns: shared state that outlives its producer is exactly the trust problem ADR 0002
  was written to avoid.
- *Marking user profiles writable.* Reintroduces the write path at the profile level,
  making writable access to *user data* — not run-scoped scratch — a configuration
  away. It requires the same conditional gate as the permit flag, and it puts that
  gate between the user and their own databases. The design's non-goal stands: no
  write to a user-registered database, ever; scratch is the only writable SQL and it
  is run-scoped, not user data ([DESIGN](../plan/DESIGN.md) §2).

## Open question recorded for amendment

U3 — DuckDB ReadWrite file-creation semantics — is resolved empirically by M4-1, not
by assumption: the pinning suite runs against the bundled engine, this ADR is amended
with what it reveals, and only then does `scratch_sql` advertise the capability. Until
that amendment lands, this ADR describes intent, not verified behaviour.

---

## Amendments

### 2026-09-10 — M4-1 pinning: decision 4 is withdrawn

`crates/saya-harness/tests/scratch_semantics.rs` ran the configuration against the
bundled engine. Three of its claims held. Two did not, and one of those is a security
finding that changes the decision rather than qualifying it.

**Held.** ReadWrite creates a missing database file; it does not create missing parent
directories; DDL and DML round-trip; `read_csv` on a local file works while external
access is on; `lock_configuration` refuses `SET enable_external_access` in *both*
directions after open; an `https://` URL is unreadable with no extension loaded.

**Did not hold — file mode.** DuckDB creates the database file at **0644**, group- and
world-readable. The run directory is 0700, so nothing reaches it today, but the file
carries no protection of its own: whatever widens the run directory exposes every
staged row. A `chmod` after create is the caller's job, and the ADR should not have
assumed otherwise.

**Did not hold — extensions, and this withdraws decision 4.** The claim was that
`enable_autoload_extension(false)`, community-extension denial and `lock_configuration`
keep extensions out while `enable_external_access(true)` lets corpus files load. All
three miss:

- `httpfs` is a **core** extension, so `allow_community_extensions = false` never
  applies to it;
- autoload governs *implicit* loading, not an explicit `INSTALL` or `LOAD`;
- `lock_configuration` locks settings, and `INSTALL` is not a setting.

Measured: `INSTALL httpfs` succeeds — reaching DuckDB's extension repository over the
network to do it — `LOAD httpfs` succeeds, and the connection then makes outbound TCP
connections. **That is an egress path that never crosses the fetch policy (M3-1).** A
`scratch_sql` shipped on decision 4 would have made SSRF-by-SQL available to any run
holding the `scratch` scope, with the fetch allowlist intact and bypassed. The
discriminator is pinned as a test on the error *kind*: with external access on the read
fails at the network layer, with it off the same statement fails at DuckDB's permission
layer and never leaves the process.

**Therefore decision 4 is withdrawn and the recorded fallback becomes the decision:
`enable_external_access(false)`.** Measured, that refuses `INSTALL`, `LOAD`, an http
read *and a local file read*, all at the permission layer — the flag is all-or-nothing;
there is no "local files yes, network no" setting to reach for. So the fallback is not a
milder decision 4, it is the whole of it: **no `read_csv` inside the engine at all**, and
corpus loading happens outside it, through the workspace tools that are already
contained and already bounded. DDL, DML and joins on the scratch file itself are
unaffected, which is the capability the ADR was written to get.

The narrowed-hardening review that decision 4 asked for is moot — there is nothing
narrowed left to review. What M4-2 now needs sign-off on is the *ingestion* path that
replaces `read_csv`.

**Entry condition for M4-2, restated.** `scratch_sql` opens with external access off; its
validator rejects any file-reading function outright rather than canonicalising a path
argument; the property battery asserts the permission-layer refusal, not a path check.