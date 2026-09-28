# ADR 0004: Saved investigations, execution evidence, and export modes

- Status: accepted 2026-09-28. Records what release 0.4.2 shipped, as the
  Milestone A decisions D1–D11 of the usability plan, revised where the
  implementation revised them (decision 7).
- Date: 2026-09-28
- Supersedes: nothing. Complements [ADR 0002](adr-0002-memory-and-contract-trust-model.md)
  (what never reaches a provider) and [ADR 0003](adr-0003-scratch-database.md)
  (the one writable SQL surface — saved investigations are not it).
- Records why a saved investigation is a portable JSON document rather than a
  row in the state store, why replay never constructs an AI provider, what the
  evidence and capture types are for, and what this release still does not do.

## Context

Two usability gaps motivated Milestone A. First, a good answer is
disposable: a question answered well yesterday — with the SQL that produced
it — cannot be replayed tomorrow or handed to a colleague, so the same
question is re-derived from scratch (and re-billed) every time. Second, an
answer carries no provenance: a table of rows says nothing about which
profile ran it, when, or with what statement, so nothing about the result can
be checked or shared with confidence.

Both gaps are closed without touching the product's central invariant: no
write to a user-registered database, ever (ADR 0003). Saving an investigation
is a file write on the user's own machine; replaying one crosses the same
read-only gate every query crosses. Neither feature grants SQL a capability
it did not have.

## Decision

### 1. A saved investigation is an explicit, portable, versioned JSON document (D1)

`InvestigationDefinitionV1` lives in
[`saya-types::investigation`](../crates/saya-types/src/investigation/mod.rs):
format `"saya.investigation"`, version 1, and fields — id, revision, name,
description, the exact SQL, the dialect, the logical connection alias, the
referenced objects, an optional schema fingerprint, and created/updated unix
ms. The document holds no credentials, no rows, no results, no prompts, no
transcripts, no grants, and no machine-specific profile identity — a person
who receives the file learns what runs, not who ran it or where they work.

Bounds are enforced in
[`validate.rs`](../crates/saya-types/src/investigation/validate.rs) for every
definition however it arrives: 128 KiB per document, 64 KiB of SQL, an
80-character name, a 2 KiB description, 256 referenced objects. An unknown
major version is refused (`UnsupportedVersion`), never silently downgraded,
and unknown JSON fields are refused rather than skipped — a document written
by a newer saya cannot be half-read by an older one.

Ids are derived deterministically (slug of the name plus 8 hex characters of
a SHA-256 over name, SQL, and creation time —
[`id.rs`](../crates/saya-types/src/investigation/id.rs)), so an id is always
a valid filename stem. Filenames never come from display names or from
imported paths.

### 2. Storage is one JSON file per id, not the SQLite state store (D2)

[`saya-store::investigations`](../crates/saya-store/src/investigations/mod.rs)
owns the I/O: one file per id at `<root>/<id>.json`, file mode 0600 inside a
0700 directory, published through the existing private temp-file + fsync +
rename seam (`replace::publish_staged`). No migration of the main state
store, and the portable document never mixes with per-machine state.

The root is `SAYA_INVESTIGATIONS_DIR` when set, else
`<platform data dir>/saya/investigations` — the same resolution chain as the
state database ([`paths.rs`](../crates/saya-cli/src/commands/investigation/paths.rs)).

Optimistic revision control: `create` refuses an existing id; `update`
requires the on-disk revision to equal the caller's expected revision; delete
checks the revision the same way. A corrupt or unsupported file is reported
(`list` surfaces it as a warning line, `get` refuses), never overwritten and
never deleted implicitly.

The collection caps at 500 documents, and `list` is a bounded scan returning
at most 50 per page with `--offset` paging past it.

Per-machine state lives beside the document, never inside it:
`<root>/local/<id>.json` holds the local binding — profile name, opaque
profile identity, reviewed revision, reviewed schema fingerprint, reviewed
time. It is never exported; the export path reads the definition only.

### 3. Saving validates, never executes (D3)

`investigation save` takes the exact SQL (from `--sql`, `--file`, or piped
stdin) and a resolved profile. The SQL is checked by
`saya_connectors::prepare_for_dialect` — the same gate execution uses — for
the profile's dialect; multi-statement or write SQL is refused. Nothing
connects and nothing runs: preparation only, against a one-row bound that is
never executed ([`save_input.rs`](../crates/saya-cli/src/commands/investigation/save_input.rs)).

Credential-shaped content — detected by `saya_types::redact` — is refused,
not redacted. Redaction would silently change what a replay runs; refusal
keeps the document's SQL semantics honest. The UI states the other half
truthfully: exact SQL is saved and must be reviewed before sharing, and no
message claims the document is proven secret-free.

Saving records a local binding to the saving profile: the author's machine
remembers that this profile reviewed exactly this revision, with this
fingerprint, at creation time.

### 4. Replay shares the query path, has no provider, and targets an explicit connection (D4)

`investigation run <id>` resolves the target only from `--connection <profile>`
or the stored local binding. It never falls back to the active or default
profile — a saved question cannot silently run against whichever database
happens to be selected. A profile whose dialect differs from the saved
dialect is refused.

Replay never constructs an AI provider. The SQL traverses the same connector
safety gate, the current row and time limits, and the read-only policy as
`saya query` — replay is a query with a remembered statement, not an agent
turn.

Review is bound to the triple (definition revision, profile identity,
fingerprint of the referenced objects). If any differs from the binding, the
run refuses — `review is stale (…); pass --revalidate to re-review the
current state` — until `--revalidate` is passed, headless included; a
revalidated run rewrites the binding. The first run after an import requires
`--connection`; that explicit mapping creates the binding, written on
success, never before.

### 5. Export and import move the definition only (D5)

`investigation export <id> <path>` writes only the portable document,
atomically (private temp + rename), refusing an existing destination unless
`--overwrite` and refusing a directory or symlink destination.

`investigation import <path>` reads at most 128 KiB + 1 (so an oversize file
is refused rather than truncated), validates the whole document, re-gates the
SQL exactly as save does, prints a preview (id, name, dialect, the
connection requirement, referenced objects, and the exact SQL), and stores
the document with no binding. Identical id and content is an idempotent
no-op; the same id with different content is a conflict. Import never
executes, never binds, never grants, and the imported review metadata carries
no authority — a binding exists only after a local run establishes it.

### 6. One operation, multiple adapters (D6)

The clap subcommands `saya investigation save|list|show|run|export|import|delete`
and the slash command `/investigation …` share one `InvestigationCommand`
enum and one operation module
([`commands/investigation/`](../crates/saya-cli/src/commands/investigation/mod.rs));
the slash adapter only fills SQL and connection from the latest successful
query when the user omitted them. Parity tests compare outputs, so the two
surfaces cannot answer differently.

### 7. Only successful, concrete queries are selectable (D7 — revised)

**Revision from the original decision:** the plan considered carrying
success/failure on `AgentEvent::ToolCompleted`; shipped instead with no
agent-event contract change. `ToolCompleted` stays `{ name, summary }`, and
success is classified by the same summary classifier the renderers already
use — `render/tool_groups.rs` `is_failure_summary` — so the transcript and
the selectable query can never disagree about what succeeded.

The TUI keeps a FIFO of pending `bounded_sql_query` requests (SQL plus the
connection, the connection defaulting to the session profile at request
time). `ToolCompleted` for that tool pops the front; it becomes the latest
selectable query only if not a failure. `ToolDenied` pops without promoting.
A rejected or failed request never replaces the previous selectable query —
a broken turn cannot overwrite a query that worked. Fan-out
(`bounded_sql_query_all`) never becomes selectable; the user picks one
connection with `/sql`.

### 8. Evidence records the execution; capture is ephemeral (D8)

`ExecutionEvidence` in [`saya-types::evidence`](../crates/saya-types/src/evidence.rs)
records one successful execution: execution id, SHA-256 of the submitted
SQL, connection label (the profile name — safe to show) plus an optional
opaque identity, dialect, optional schema fingerprint, start/finish unix ms,
row cap, returned rows, truncated, scope (full / model-limited), and source
(direct SQL / saved investigation id + revision / agent). Serialized evidence
names the statement by hash only. The field is labelled *submitted SQL* —
`executed_sql` — because saya does not claim it is the safety layer's
internally rewritten statement.

The TUI keeps at most one `CapturedResult { result, evidence }` for the
latest direct `/sql` success, in memory only, on `App` (which is never
serialized — the type has no serde derives, so captured rows cannot reach a
session file). Capture is refused visibly above the 32 MiB accounted budget,
naming the way out (`/export --refresh`). Agent-path results are not
captured: their rows are model-limited and never in the event stream, so
snapshot is unavailable there and refresh is the offered path.

The evidence line shown with a direct result reads, e.g.,
`direct sql: demo · 12 rows (truncated at 50) · exec x… · full result` —
source, profile label, rows, truncation, short execution id, and scope.

### 9. Export modes: snapshot writes what you have, refresh re-queries (D9)

`/export --snapshot <path>` writes the captured result with no query at all;
`/export --refresh <path>` re-runs the latest selectable query on its original
connection. The legacy form `/export <path>` stays a refresh and says so in
its success line, pointing at `--snapshot`. Export writes via private temp +
rename, refuses an existing destination unless `--overwrite`, and caps
encoded output at 32 MiB checked while encoding; a failed or refused export
preserves the existing destination byte-for-byte. (Compatibility note: legacy
`/export` used to overwrite silently; it now needs `--overwrite`.)

### 10. The Markdown report (D10)

`/report <path> [--rows N]` (N ≤ 100) writes a report from the captured
evidence; `saya investigation run <id> --report <path>` writes the same shape
from a replay. The default content is the SQL and provenance only — no rows,
no conversation. The report is capped at 2 MiB; links, images, HTML,
table-breaking characters, and control characters in cells are neutralised;
omitted or truncated content is
labelled. It never opens a browser and never uploads anything.

### 11. The demo and guided setup (D11)

`saya demo` creates a deterministic synthetic SQLite database — customers,
orders, and customer_contacts; 240 customers, 560 orders, and 123 contact
rows, seeded by a fixed
LCG — under `<data dir>/saya/demo/` (override `SAYA_DEMO_DIR`), written by a
fixture initializer in saya-cli over sqlx, deliberately never through
`DatabaseConnector`. The fixture plants the traps that make SQL hard: nulls
in emails and amounts, a customer_contacts table that multiplies rows on
join, order dates straddling the 2025-12-31 → 2026-01-01 boundary, and an
ambiguous "active customer": `customers.status = 'active'` and "ordered in
the last 90 days of the data" disagree for some customers, so the phrase has
two defensible meanings. It then opens the database
READ-ONLY in the TUI; no AI provider is needed for schema browsing or `/sql`.
Without a terminal it prints the paths, the launch command, and example SQL.

`saya setup` is an interactive guided flow: draft → validate → probe →
review → commit. With no TTY or `--non-interactive` it never prompts and
exits 2 with guidance to `saya config init` or `saya demo`. It writes only
user-level config, never resolved secrets (environment-variable *names*
only), preserves existing profiles and unrelated settings (connections.toml
is appended to with the existing bytes kept as an exact prefix; a config.toml
that already exists is never modified — setup prints the `[ai]` snippet for
the user to apply), backs up files privately before replacing them, and
writes a commit marker so an interrupted two-file write is detected on later
startup and offered restore/finish. Probes run one at a time, 15 s each; a
provider probe states that a request will be sent and sends only the word
"ping" — no schema, no rows.

## Consequences

**Accepted costs.**

- Saved-investigation storage is a directory scan, not an indexed table.
  At the 500-document cap and a 50-per-page bound, the scan stays bounded by
  design; a store with thousands of documents would want the state store —
  which is exactly the trade D2 rejected to keep documents portable.
- The document and the local binding are two files. Export must remember to
  take one and not the other; the export path reads the definition only, and
  the split is what makes portability real.
- The stale-review refusal asks the user to re-review on every revision,
  target, or schema change. That friction is the feature: a saved query that
  silently runs against a different table is worse than one that stops.
- D7 keeps `ToolCompleted` at `{ name, summary }`, so the selectable query
  rides the same string classifier the transcript renders. A summary format
  change is now load-bearing for selection as well as display; the classifier
  is one function both read, and the parity tests hold them together.
- Captures are session-ephemeral by construction. Snapshot and report cannot
  offer a result from before the last restart, and the docs say so rather
  than implying durability.

**Rejected alternatives.**

- *Storing investigations in the SQLite state store.* Convenient for
  indexing, but the state store is machine-local by design; a saved
  investigation is a shareable artifact, and a database row would have made
  export a conversion step with its own drift.
- *Redacting credential-shaped SQL instead of refusing it.* A redacted
  statement runs differently from the one the author wrote; a save must
  either keep the SQL exact or refuse. Refusal keeps the document's meaning
  and pushes the fix to the author, where the knowledge lives.
- *Falling back to the active profile on replay.* The investigation would
  run against whichever database the session happened to have selected — the
  reviewed target becomes a suggestion. Replay refuses instead: the target
  is part of what was reviewed.
- *Carrying success on `AgentEvent::ToolCompleted`.* A contract change
  across the crate boundary to save one string comparison; the summary
  classifier already decides what the transcript shows as failed, and the
  renderer's truth is the one the user can verify on screen.
- *Persisting captures to the session file.* Session persistence is
  redaction-hardened against rows by structure; weakening that for
  convenience would re-open the channel ADR 0002 closed.

## Limitations (stated, not solved)

- Snapshot and report capture only direct `/sql` results. Agent-run query
  results are not captured — their rows are model-limited and not in the
  event stream — so snapshot is unavailable after an agent turn; use `/sql`
  or `/export --refresh`.
- Captures do not survive a restart. Capture times in messages are UTC.
- TUI `/investigation run` runs in the foreground; the transcript waits for
  the query.
- Parameterised investigations, context import, dbt, file sources, and MCP
  are not in this release.
- The report neutralises links, images, HTML, table-breaking characters, and
  control characters, but
  a Markdown renderer may still auto-link a bare `https://…` text value in an
  included row. Review the report before sharing.
- Guided setup covers four engines (SQLite, DuckDB, PostgreSQL, MySQL);
  Snowflake, ClickHouse, and BigQuery are configured in connections.toml as
  before.
- `executed_sql` / "submitted SQL" is the SQL saya submitted to the
  connector, not the safety layer's internal rewrite.