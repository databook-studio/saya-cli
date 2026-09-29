# ADR 0006: Portable reviewed business context — `saya.context`, dbt metadata, and clarification

- Status: accepted 2026-09-29. Records what shipped for release 0.4.2's
  context-portability and dbt milestones (B2, B3a), verified against the
  code; where the plan and the code differ, the code is recorded. The
  clarification milestone (B3c) also merged on 2026-09-29; §6 records what
  shipped.
- Date: 2026-09-29
- Supersedes: nothing. Complements [ADR 0002](adr-0002-memory-and-contract-trust-model.md)
  (claims are confirmed facts, never authority to execute) and
  [ADR 0004](adr-0004-saved-investigations-and-evidence.md) (portability of
  definitions; this gives the *reviewed knowledge* the same property).
- Records why confirmed claims travel as a document, why imports always land
  Pending regardless of what the document claims, how dbt manifests feed the
  same import path without running dbt, and what remains open.

## Context

Confirmed contracts — table descriptions, column meanings, time columns,
join rules — are the part of saya that makes an answer correct, and they are
exactly what the 0.4.2 memory model keeps machine-local. A colleague who
inherits a workspace starts from nothing and re-derives every fact the first
author already had confirmed. The facts are not sensitive the way credentials
are; what must not travel is *authority* — an approval by someone the local
store never heard of must not arrive as a confirmed fact.

dbt already encodes much of this: model and column descriptions, and the
`relationships` generic test that says one column references another. That
knowledge exists at rest in `manifest.json`; reading it requires no dbt
process. And both channels share one failure mode to design against: the
document (or the manifest) is untrusted input that claims things about
schema objects that may not exist here.

## Decision

### 1. The document: `saya.context` v1

[`ContextDocumentV1`](../crates/saya-types/src/context_doc/mod.rs) — format
`"saya.context"`, version 1, `exported_unix_ms`, and `items` — is bounded at
three levels, all enforced wherever it is read or written: 1 MiB per
document (checked on the raw bytes before any parsing, and again on
serialized output), 500 items, 4 KiB per item (the item is serialized and
measured), plus a 256-byte control-character-free cap on the optional
`origin_note`.

Each item pairs a logical target with a payload:

```json
{ "object": { "catalog"?, "schema"?, "name", "kind" },
  "payload": { "kind": "claim", "claim": { …ClaimPayload… } },
  "origin_note"? }
```

The [`PortableObject`](../crates/saya-types/src/context_doc/mod.rs) is a
logical name — catalog and schema optional so the importer resolves them
against the destination profile's schema — and the
[`PortablePayload`](../crates/saya-types/src/context_doc/payload.rs) is a
claim-shaped payload (the existing validated `ClaimPayload` serde), a
relationship, or a join rule. Target-bearing payloads cannot ride inside a
plain claim (refused at conversion). The document carries what a claim *is*,
never who reviewed it or which machine it came from: no profile identity, no
review state, no evidence, no credentials, no rows, no grants, and no
timestamp beyond `exported_unix_ms` — pinned by a test asserting the
serialized JSON contains none of the identity or review markers.

### 2. Export moves that profile's Active claims only

`saya contracts export <path> [--profile P] [--overwrite]` writes one
document containing the **Active** knowledge items of that profile — nothing
Pending, nothing dismissed, nothing from another profile. Claims that cannot
be made portable are counted as `skipped`, never silently dropped. The write
is atomic (private temp + fsync + rename), refuses a symlink or directory
target even with `--overwrite`, and refuses to overflow the 500-item cap
rather than truncating. `origin_note` is exported as absent — provenance
notes are produced by the dbt path, not by export.

### 3. Import maps, quarantines, and commits atomically

`saya contracts import <path> --profile P [--preview]` runs in one order,
and the order is the safety property:

1. **Validate the whole document first**, before any store access — bounds,
   schema, every payload through its validating constructors. A malformed
   document writes nothing by construction.
2. **Map each item to this profile's schema** — the CACHED tree when
   non-empty, else a live fetch through the connector, and the report names
   which (`cached` / `live`). Resolution is **case-insensitive** and
   refuses to pick among several candidates: portable objects carry no
   quoting, so dialect folding could not be applied soundly; a resolved
   candidate keeps the tree's spelling. Unresolved or ambiguous items are
   reported as **unavailable** — never written.
3. **File every mapped item as Pending**, with origin `team_file` —
   regardless of any approval the document claims. The store's batch helper
   has no way to write an Active row; the document's approvals carry no
   authority (ADR 0002's trust model, applied to a file).
4. **Credential-shaped items are skipped, not fatal.** The store's batch
   admission refuses any credential-shaped payload, and that refusal would
   abort the whole batch — so the mapping applies the same credential-shape
   predicate per item and reports the item (`credential-shaped text`) as
   unavailable while the rest of the import proceeds. The store's admission
   check stays behind it as the backstop; any *other* store-admission
   refusal still aborts the import (all-or-nothing holds there).
5. **Conflicts are reported, not written.** A mapped item whose single-valued
   slot already holds a *different* value locally (Pending or Active) is a
   conflict; a **dismissed** row is likewise a conflict and is never
   resurrected — the user explicitly forgot that fact, and an arriving file
   does not undo the forgetting. There is no order-based winner: the local
   value stands and the import reports the conflict for the user to decide.
   An item identical to the local value is **skipped**, which makes
   re-import idempotent.
6. **Commit all-or-nothing.** All writes go through one store transaction
   ([`apply_pending_batch`](../crates/saya-store/src/knowledge_items/batch.rs));
   a storage failure mid-batch rolls back everything, leaving the store
   byte-identical.

`--preview` runs steps 1–2 and reports without opening a transaction.
Memory mode `off` still allows import — claims land Pending and recall stays
off — and the report says so rather than implying the facts took effect. An
import where items were offered but none landed exits 2.

`/contracts export|import|import-dbt` in the TUI parse into the same
`ContractsCommand` enum and run through the same operation (parity-tested);
the slash surface has no `--profile` — it always targets the active profile.

### 4. dbt metadata feeds the same import path

`saya contracts import-dbt <manifest.json> --profile P [--select <glob>…]
[--preview]` reads the manifest with the same bounded-read discipline: at
most 32 MiB (declared size checked, then a capped read, so a file that grows
mid-read cannot exceed), manifest schema version **v10, v11, or v12** only
(probed from `metadata.dbt_schema_version` before full parse — anything else
is refused naming the version), and at most **5,000 selected nodes**
(refused, never truncated). `--select` globs match node names;
**models and sources only** — seeds, snapshots, and every other node kind
are out of scope.

Mapping is exactly three things and nothing else: node description →
table description; column description → column description; the generic test
whose `test_metadata.name == "relationships"` (`kwargs.to` / `kwargs.field`,
the tested column) → a relationship. Nothing is executed: no dbt process,
no compiled SQL, no macros read, no Jinja evaluated — the fixture tests
plant macro markers in manifests and assert none leak into items.

The manifest's text is untrusted data, and existing validators decide: every
payload goes through the same validating constructors as interactive input,
and text that is oversize or carries control characters fails **per item**
— skipped and counted with a stable reason (`table_description_invalid`, …),
never loosened and never fatal to the whole import. Provenance is recorded
as `origin_note = "dbt <version> <unique_id>"` (the manifest's
`dbt_version`, else `"unknown"`). A relationship's `ref()`/`source()` target
is resolved **name-only against the selection**; an unparseable, missing, or
ambiguous target is counted (`unresolved_target` / `ambiguous_target` /
`malformed_target`) — the safe failure, not a guess.

Output flows through §3's import path unchanged: everything lands Pending
with origin `team_file`, conflicts are reported, the commit is one
transaction, `--preview` writes nothing.

### 5. Relationships file as keyed join rules

There is **no Relationship slot** in the store. A relationship files as a
`relation.join_rule` row — target, local columns, target columns, and a
condition string built from the positional column equalities (`customer_id =
id`). Join rules are a multi-valued slot (at most 4 per table). The
document carries a cardinality (the dbt mapper hardcodes `many_to_one`, the
only thing a `relationships` test asserts), and the cardinality is **dropped
at filing time** — nothing in the store consumes it, so the ADR records that
rather than implying it survived.

### 6. Clarification — shipped (B3c)

When a question's material definition is ambiguous and the confirmed context
does not resolve it, the agent may ask one focused question instead of
assuming. The model reaches the question through the `request_clarification`
tool — `{ question, options[≤6]? }`, deliberately **effect-none**: it ends
the turn rather than returning data. The turn ends with a structured
`AgentEvent::ClarificationNeeded`; the TUI renders it as a Question block
(and flushes any buffered tool-call group first, so the block reads in
order), and headless `saya ask` exits `6` — the paused class, "needs input" —
with the `clarification_needed` event on the JSON/NDJSON surface, so a script
can tell "needs input" from "answered". The system prompt carries the
guidance: ask rather than assume when a material definition is ambiguous.
The tool is always advertised (it needs no data sharing, since it returns
nothing) and its answers are never fed back automatically — the human
answers the question in their own words next turn.

## Consequences

**Accepted costs.**

- Imported context is double work until reviewed: an importing team re-reviews
  what an exporting team already confirmed. That friction is the feature —
  the alternative is a file's approval claims becoming a local authority
  nobody locally examined.
- Case-insensitive resolution means two objects differing only by case
  cannot both be targets of one item — the ambiguity is reported instead of
  guessed. (Portability across engines was judged worth more than
  case-sensitive precision; portable objects carry no quoting.)
- The dbt mapper is deliberately narrow. Descriptions and the one generic
  test that encodes a join are in; metrics, aliases, seeds, snapshots, and
  every other test kind are not — and `relationships` is the only generic
  test whose shape has a portable meaning.
- Per-item skips mean a manifest with junk text produces a partial import
  that must be read in the report, not an error to fix and retry.

**Rejected alternatives.**

- *Importing as Active when the document says the item was approved.* An
  approval is a local act by a local reviewer; a file asserting otherwise is
  exactly the authority-in-a-file problem ADR 0002 was written against.
  Every import is Pending, with no path around it.
- *Merging the imported document with local state at import time
  (last-writer-wins).* Silently choosing between two different time columns
  or two different join rules is a correctness bug wearing a convenience.
  Conflicts stop and wait for the user.
- *Running dbt to generate metadata.* The manifest is the artifact; running
  a pipeline as a side effect of a metadata import adds an execution surface
  and a version matrix to a read-only product for nothing the manifest
  doesn't already contain.
- *A dedicated Relationship slot.* Cardinality sounded like information and
  turned out to be decoration: nothing consumes it, so filing relationships
  as join rules keeps one shape (the keyed equality) instead of two.

## Limitations (stated, not solved)

- The document is capped at 500 items and 4 KiB each; larger estates need
  multiple documents (each imported as its own transaction).
- dbt `relationships` targets resolve only against the *selection* — a test
  on a selected model referencing a non-selected target is counted as
  unresolved, not followed outside the manifest slice.
- Cardinality is dropped at filing; the document carries it, the store does
  not.
- Import maps against cached-or-live schema per profile; it never merges
  across profiles, and `/contracts import` (slash) always uses the active
  profile.
- Only credential-shaped items are skipped per item; every other
  store-admission refusal aborts the whole import.
- The deterministic correctness scenarios (duplicate-join trap, time-column
  choice, the "active" ambiguity) exist as fixture data and a manual
  walkthrough, not as scripted-provider tests.