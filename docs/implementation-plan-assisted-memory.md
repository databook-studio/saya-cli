# Implementation plan — assisted memory

Supersedes the demo-only scope of Phase 7, and supersedes this document's own first draft, which
sequenced the work wrongly. See "What the first draft got wrong" below — it is kept because the
reasoning that produced it will otherwise be reproduced.

## The promise

> SAYA accumulates bounded suggestions while you work. Every turn shows exactly which saved claims
> were supplied to the model, and every new claim persisted. Unconfirmed suggestions never gain
> authority merely by being recalled.

Four words are deliberately distinct and must stay distinct in code, events, docs and copy:

| Word | Means |
| --- | --- |
| **persisted** | written to the local store |
| **supplied** | placed in the model's context this turn |
| **acknowledged** | shown to the user |
| **used** | actually influenced the generated SQL — *which we do not measure, and therefore never claim* |

## What the first draft got wrong

1. **It flipped both defaults in Slice A.** ADR 0002's acceptance rubric gates
   `learning = "auto-candidate"` on candidate precision, "measured from Phase 3". That measurement
   has never been taken. The draft removed the gate without replacing it.
2. **It shipped visibility after the answer.** By then the candidate has already shaped the SQL and
   the query has run. The user could correct turn N+1 but not prevent turn N. A receipt that arrives
   after execution is a changelog, not a control.
3. **It proposed counts, not claims.** "2 facts" does not tell you what was assumed. It also calls
   an unconfirmed proposal a fact, and asserts causal use.
4. **Its natural-language correction demo could not work.** Every model proposal is hardcoded
   `AssistantInferred` + `Candidate` (`propose/mod.rs:89`). "No, rentals count against rental_date"
   produces a second candidate and a conflict, not a correction.
5. **It deleted the deterministic tapes.** They are the only reproducible regression coverage of the
   contracts surface; a model-driven recording cannot replace them.

## The semantic contradiction to resolve before candidates are recallable

`contract_propose`'s description — text sent to the model — says a candidate is
"inert until a human confirms it" (`propose/definition.rs:40`). Default `IncludeCandidates` makes
that false: a candidate would shape the next query. Either keep candidates out of default recall, or
amend the trust model to "a candidate is an **advisory assumption** that may influence SQL" and
change that description with it. Shipping both sentences as they stand ships a lie to the model.

## P0 — recall does not fire for ordinary questions

`selection.rs:178` matches a term only if it is a substring of the qualified name, so the plural
`rentals` never matches table `rental`. Verified live, same store, same confirmed claim, no hint:

| Prompt | Recall | SQL |
| --- | --- | --- |
| `How many rentals were there in each month of 2022?` | silent | `rental_date` — claim ignored |
| `How many rows are in pagila.public.rental for each month of 2022?` | fires | `return_date` — claim applied |

Everything downstream is sound; selection is broken. Precision measured on this selector measures
nothing, so this gates Phase 5's evaluation.

## Decided: a confirmed claim is binding

A confirmed claim is a **binding instruction**, not advisory context, and a deviation must be
surfaced rather than buried. A candidate stays advisory — the asymmetry is the point.

This was decided on evidence, not principle. Live against pagila, deterministic across three runs
each, with a confirmed `user_explicit` `default_time_column = return_date` on `pagila.public.rental`:

| Prompt | Executed |
| --- | --- |
| `How many rentals were there in each month of 2022?` | `WHERE return_date` |
| … + `Show me the SQL.` | `WHERE rental_date` |

The model saw the claim and discarded it, explaining: "I used `rental_date` … rather than
`return_date` (the contract's default time column), since you asked about rentals per month." The
reasoning is not stupid — which is exactly the problem. Nothing in the prompt said it may not
silently substitute its own judgement, because `render.rs` gives a confirmed claim **no marker at
all**: candidates get `[candidate — unconfirmed]`, disputes get `[disputed]`, and a confirmed fact
renders bare, formatted as context and therefore treated as context.

ADR 0002's rubric would not have caught this. It forbids *unsupported* and *stale* memory use, both
"must be zero", but has no metric for **supplied-and-overridden**. Add one.

### Measured: a prompt directive surfaces an override but cannot prevent one

P2a shipped a stanza directive ("Confirmed claims below bind: use them as given, and say in the
answer when you depart from one") plus a `[confirmed]` marker, with `[disputed]` suppressing it so
two contradictory confirmed claims never both read as instructions. Gate green, 926 tests.

`scripts/memory-acceptance.sh`, three runs per prompt, before and after:

| Prompt | Before P2a | After P2a |
| --- | --- | --- |
| `How many rentals were there in each month of 2022?` | 3/3 bound | 3/3 bound |
| … + `Show me the SQL.` | 3/3 overridden | **3/3 overridden** |

What did change is the honesty of the override. The answer now leads with "**Departure from a
confirmed claim:**" instead of burying it in a trailing note. So the directive achieves *surfacing*
and not *binding* — a prompt instructs, and this is the measurement proving it cannot enforce.

**The model's objection was also correct**, which matters for the design: `return_date` is NULL for
unreturned rentals, so the claimed rule undercounts. Blind enforcement would produce quietly wrong
numbers. Whether a contradiction should *block* an answer or merely be *reported* is therefore a
product decision, not an implementation detail — and detection is a prerequisite either way.

Two slices, in this order, because the second measures the residue of the first:

- **P2a — authority in the prompt.** Confirmed claims render as binding; candidates unchanged.
- **P2b — structural override detection.** `saya_connectors::safety::sql_references` already returns
  the objects and columns a statement references, with an explicit `partial` flag meaning "at least
  these, possibly more". Compare that against the confirmed claims the P1a receipt says were
  supplied. **A false override warning is worse than a missed one** — it trains users to ignore the
  signal — so an unparseable or `partial` result must stay silent rather than accuse.

`scripts/memory-acceptance.sh` is the gate: it establishes the claim in an isolated `HOME`, asks the
real model through the real gateway, and reports the column each answer actually filtered on. It
greps the executed `WHERE` predicate rather than any mention of a column name, because the model
names both while explaining itself and a looser match reports false passes.

## Phases

**Phase 0 — trust contract and evaluation.** Fix the four-word vocabulary above. Decide inert vs
advisory. Define upgrade vs new-install behaviour. Set candidate-precision, recall-precision,
correction-rate and SQL-regression gates.

**Phase 1 — typed memory receipts.** `RecallReceipt` returned beside context blocks, carrying claim
id, safe profile name, object, kind, value, status, schema state. `KnowledgeSupplied` emitted
**before the provider request** (`runtime.rs:117`), remaining visible with the answer. Render tests
across text, JSON, NDJSON, REPL and TUI.

**Phase 2 — visible persistence.** `KnowledgeProposed` after successful persistence, carrying the
proposed value — today the TUI shows `→ contract_propose` with no value, and completion reports
"read-only database tool completed" for a tool that writes local state. Bounded per-turn and
per-profile pending-candidate limits.

**Phase 3 — in-flow decisions.** Shared `confirm_candidate`, `reject_candidate`, `correct_claim`,
`use_candidate_once`. Expandable TUI memory card with direct actions. Natural-language decisions
become model-*proposed* actions requiring explicit confirmation showing before/after. **The model
must never infer that ordinary agreement confirms durable knowledge.** Fingerprints and
referenced-column snapshots revalidated transactionally.

**Phase 4 — anti-feedback and safety.** A query influenced by a recalled claim must not become
independent evidence for that claim. Carry supplied claim ids in a turn-scoped receipt; a duplicate
proposal for a claim supplied that turn earns no evidence unless the user explicitly confirmed it or
the evidence came from an independent structural check. Validate proposed objects and columns
against fresh schema before storage. Confirm no memory action can widen SQL approval or safety
policy.

**Phase 5 — controlled rollout.** `learning = auto-candidate` with `recall = confirmed`. Measure.
Enable candidate recall only if the gates pass. Assisted defaults apply to **new installs**;
existing installations that omitted `[memory]` keep `confirmed`/`off` and get a one-time activation
card. Config absence cannot distinguish new from upgraded, so this needs a persisted policy version
or onboarding marker. Explicit configuration always wins.

**Phase 6 — documentation and demos.** Amend ADR 0002; rewrite `docs/memory.md` and
`docs/announcement-memory.md`. One hero TUI recording of two real sessions with natural-language
correction. Keep the deterministic CLI tapes for lifecycle, drift, refusal, forget and team import.
A headless acceptance test proves the persisted correction changes both supplied memory and
generated SQL — **the video is recorded only after that test passes.**

## Carried-forward non-negotiables

- Read-only by default; no bypass of `saya-connectors/src/safety/`.
- No secret read, typed or recorded; saya loads its own key via `--env-file`.
- The opaque profile identity never reaches rendered output.
- No absolute path, username, hostname or token in any recorded frame.
- Behaviour changes start with a failing test.
- One operation, multiple adapters: every knowledge event renders in TUI, REPL, text, JSON, NDJSON.
