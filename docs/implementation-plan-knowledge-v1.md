# Implementation plan — knowledge engine v1

Supersedes `implementation-plan-assisted-memory.md` for everything not yet built. Written after an
architecture audit found the subsystem overbuilt relative to the loop it delivers, and verified
against `HEAD`.

## The decision that shapes everything: nothing has shipped

There are no deployed stores, no installed base, no users. That removes the expensive half of a
normal restructuring:

- **No migration.** No `Candidate → Pending` mapping, no compatibility period, no dual writes, no
  rollback plan. Change the schema.
- **No strangler pattern.** It is a technique for live systems. With zero deployments, replace.
- **No new-install-versus-upgrade marker.** There is no upgrade path to distinguish, so `assisted`
  is simply the default.

It also changes what "overkill" means. We cannot validate against user need, so the test is: *does
this earn its place in the loop we are shipping?* What does not is **unvalidated speculation** —
worse than overkill, because it hardens into constraints the moment we ship.

## Verified findings this plan acts on

| Finding | Status |
| --- | --- |
| A confirmed claim is overridden by the model 3 runs of 3 | measured live |
| `Contradicted`, `SchemaObserved`, `QueryObserved` have no construction site | verified |
| `get_claim` pairs the claim's fingerprint hash with the **object row's** version, which is overwritten on every upsert | verified — latent, `FINGERPRINT_VERSION` has never moved |
| `suggest` mode reports touched objects, never a proposal | verified; the code's own docs say so |
| Preferences are persisted and nothing in the agent path reads them | verified |
| Learning fires only if the model volunteers `contract_propose` | verified |

## What is cut from v1

Not deleted from history — removed from the shipped surface, because no user has seen them and each
is a separate product:

| Cut | Lines | Why |
| --- | --- | --- |
| Team contract files (import/export) | ~778 | Reviewed org policy is a different product from conversational memory |
| Preferences | ~1,123 | Persisted; nothing reads them |
| Relationship claims | — | Cannot be expressed by the CLI or file format; skipped on export |
| Conflict projection, reconciliation | ~480 | Subsumed by slot cardinality (Phase D) |

## What must survive — the audit's own list, and today's commits

Profile-scoped identities · typed values · schema availability and freshness distinctions ·
dependency validation before model use · prompt-context byte and item bounds · untrusted-context
labelling · provider data-sharing gates · receipts naming exactly what reached the model · the
read-only SQL safety layer · graceful degradation when the store is unavailable.

Every commit made today sits in that list: the recall term fix, the receipt, the binding directive,
the override detector, `KnowledgeSupplied` / `KnowledgeProposed`, anti-self-reinforcement, and
in-flow confirm/reject. None is governance machinery; all of it is the visibility layer.

## Phases

Ordered by what breaks the product, not by what is architecturally tidiest.

### A — Authority (first, because no schema choice fixes it)

A confirmed claim currently loses to the model 3/3, with the model announcing the override. P2a
proved a prompt directive cannot enforce; `detect_overrides` exists and nothing calls it.

- **A1** — wire `detect_overrides` into the turn, emit `KnowledgeOverridden`, render it in every
  adapter. The user sees "SAYA used X where you specified Y" from the SQL itself, not from the
  model's confession.
- **A2** — natural-language correction: "no, use `payment_date`" resolves against the claims named in
  the current receipt and applies a correction, without an id or a subcommand.

### B — Cut scope (parallel with A; pure deletion)

- **B1** — remove team import/export from the shipped surface.
- **B2** — remove preferences from the shipped surface.
- **B3** — delete `Contradicted`, `SchemaObserved`, `QueryObserved`.

### C — The fingerprint defect

- **C1** — a claim must be decoded with the version it was written under, never the object row's
  current one. Regression test outlives whatever the storage shape becomes.

### D — Collapse the domain (the real simplification)

- **D1** — typed `KnowledgeSlot` with declared cardinality. `grain`, `default_time`, `column.role`
  single-valued; aliases and descriptions bounded multi-valued. A correction **replaces** a
  single-valued slot atomically. This is what removes conflict detection, `Contradicted`, dedup and
  reconciliation — one change, four subsystems.
- **D2** — three persisted states: `Pending`, `Active`, `Dismissed`. Validity (`Valid`,
  `NeedsReview`, `Invalid`, `SchemaUnavailable`) computed at read. Stop persisting `Stale`.
  **Must preserve** the properties review items #28 and #31 fixed: a computed-stale contract must not
  reach the model, and confirming must revalidate.
- **D3** — one `knowledge_items` projection. Object identity inlined. Fingerprint version stored
  with its binding. Evidence reduced to bounded counters, if kept at all.
- **D4** — dependency-scoped validation: a `default_time_column` claim depends on that column, not
  on a whole-table fingerprint. An unrelated new column must not invalidate business meaning.

### E — One public mode

- **E1** — `[memory] mode = "assisted" | "off"`, default `assisted`. The recall/learning axes move
  to advanced config or disappear. The independent axes are what created the matrix users should not
  have to reason about.

### F — Harness-owned learning

- **F1** — a bounded turn record and a structured extractor, so learning does not depend on the
  model volunteering a tool call. The model proposes meaning against turn-scoped object ids; SAYA
  assigns identity, schema binding, source and state.

## Sequencing and verification

A and B run in parallel — disjoint files. C is independent. D is sequential within itself and lands
after A. E follows D. F is last and is the largest single design change.

**Gate policy for this push:** subagents self-verify with `cargo test -p saya-cli --lib` (~2s); the
full workspace gate — fmt, clippy `-D warnings`, workspace tests, doc tests — runs **once at the
end** rather than per slice, to maximise throughput. The risk is that a defect compounds across
slices; the mitigation is that every diff is still read before commit.
