# ADR 0007: File sources — `saya open` stages one file into a private snapshot

- Status: accepted 2026-09-29. Records what shipped for release 0.4.2's
  file-source milestone (C1 CSV, C2 Parquet), verified against the code;
  where the plan and the code differ, the code is recorded.
- Date: 2026-09-29
- Supersedes: nothing. Complements [ADR 0003](adr-0003-scratch-database.md)
  (the one writable SQL surface — a staged snapshot is neither scratch nor a
  connector) and [ADR 0002](adr-0002-memory-and-contract-trust-model.md)
  (nothing about the file's contents reaches a provider).
- Records why a local CSV or Parquet file is staged once into a private
  DuckDB snapshot keyed by its content hash and parse contract, why the
  session over that snapshot can never open another file, and what this
  deliberately does not do.

## Context

A lot of data worth asking about is a file: a CSV export, a Parquet extract.
The obvious integration — let the session's SQL read the file with
`read_csv`/`read_parquet` — is the one thing the product must not do: it
would put a file-system and (with `httpfs`) a network reader behind the
agent's SQL tool, against a policy whose entire value is that no SQL
reaches outside the connected database ([ADR 0003](adr-0003-scratch-database.md)).
Mounting the file as a `DatabaseConnector` is no better — it would make a
local file look like a user-registered database and drag the connector's
credential machinery into a machine that has none.

What is wanted is the read-only session, pointed at a *copy*: the file read
once, under containment, staged into a private DuckDB database, and opened
through the normal read-only connector path — after the session's gate has
been tightened to refuse every file-reading function, so the staged snapshot
is the only data the session can see.

## Decision

### 1. One file, read once, contained

`saya open <file>` ([`open/mod.rs`](../crates/saya-cli/src/open/mod.rs))
reads the file exactly once through the workspace containment primitives,
not `fs::read`: the parent directory is opened as a `Workspace` solely to
open that one final component — [`read_for_scratch_import`](../crates/saya-harness/src/workspace/contain.rs)
— with every open seam carrying `O_NOFOLLOW | O_CLOEXEC`
([`fd.rs`](../crates/saya-harness/src/workspace/fd.rs)), a no-follow
`fstatat` refusing a symlink and a non-regular file, the 32 MiB cap checked
before any byte is read, and a post-open identity re-check
(dev/inode must match what `stat` saw — [`open_verified`](../crates/saya-harness/src/workspace/anchor.rs)).
Nothing else in the parent is listed or opened. The SHA-256 is streamed in
the same single open, so hash and bytes are one observation of one file.

Format routing is by the `.parquet` extension (case-insensitive) or the
file's `PAR1` magic ([`stage_source`](../crates/saya-harness/src/file_source/mod.rs)):
Parquet if either matches, every other byte stream parsed as CSV. The
routing is honest in the refusing direction: a file *named* `.parquet` whose
bytes are CSV is refused, never parsed as CSV. CSV-only flags
(`--delimiter`, `--no-header`, `--typed`) are refused on Parquet before
staging, and again when PAR1 magic contradicts the file name.

### 2. The private snapshot

The staged copy lives at `<data dir>/saya/files/<sha256[..16]>/source.duckdb`
(`SAYA_FILES_DIR` overrides the root; the default sits beside the state
database). The snapshot directory is created as a private 0700 temp dir and
renamed into place only after staging succeeds; the database file is 0600.
The staging writer lives in `saya-harness`
([`file_source/`](../crates/saya-harness/src/file_source/mod.rs)) and never
touches `DatabaseConnector` and never scratch: CSV staging writes to a
unique temp `.duckdb` inside the destination dir, commits, renames, and
removes the temp file **and its WAL** on any failure — no partial
`source.duckdb` is ever left behind. The table name is the file stem
sanitised to `[a-z0-9_]{1,63}` (lowercased, non-alphanumerics to `_`, a
leading digit gains `t_`, empty becomes `source`), with the reserved
metadata-table name `saya_file_source` refused as a user table. A metadata
table records file name, full sha256, byte size, row count, column names,
staged time, and format, in the same transaction.

### 3. The session is read-only, and its gate refuses every file-reading function

The snapshot is opened through a generated connections file with one
read-only DuckDB profile — `file_<sanitised stem>` — and the TUI launches
exactly as `saya demo` does; without a TTY the paths, preview, and launch
command are printed. The preview and the `--list` display carry the source
file name, a sha256 prefix (the snapshot *directory* is the first 16 hex
characters; the display shows the first 12), and the staged time, so a
result always names the exact bytes it came from.

Before any session opens, the read-only gate must be able to promise that
the snapshot is the only thing visible. The DuckDB policy
([`read_only_policy.rs`](../crates/saya-connectors/src/safety/read_only_policy.rs))
refuses every file-reading function: the `read_` and `http_` prefix
families — `read_csv`, `read_parquet`, `read_json`, `httpfs` reads, and
anything spelled with those prefixes — plus the prefixless readers
`parquet_scan`, `parquet_metadata`, `parquet_schema`, `parquet_file_metadata`,
`parquet_kv_metadata`, `sqlite_scan`, `postgres_scan`, `mysql_scan`, `arrow`,
`arrow_scan`, `iceberg_scan`, `delta_scan`, `glob`, and `metadata`. The
matcher compares **each identifier part**, lowercased and unquoted, so
schema-qualified (`parquet_tool.parquet_scan`) or re-quoted spellings cannot
bypass, and it is applied to every function position — scalar calls, table
functions in `FROM` and `LATERAL` — with a fail-closed whole-name rule for
plain relations. End-to-end: a session over a staged CSV that runs
`read_csv('<secret>')` exits 4; the same over a staged Parquet running
`read_parquet('/etc/passwd')` exits 4 with the function named.

### 4. CSV: stored as text, typed only where inference is unambiguous

CSV staging uses the harness's bounded RFC 4180 parser — 32 MiB source,
500,000 rows, 512 columns, 64 KiB fields; UTF-8 required; unsafe delimiters
refused; a ragged data row refuses the whole staging rather than being
coerced. Every column is stored as **VARCHAR** — no type is guessed at
storage time — and an empty field stages as NULL, matching the preview's
null counts. The 30-second staging wall clock bounds CSV staging too,
checked per insert batch.

The preview reports the delimiter (sniffed from the first 64 KiB when not
given), whether a header was used, column names, per-column null counts
(over **every** data row), and an **inferred** type per column — integer,
decimal, boolean, date, timestamp, or text — decided from at most the first
**1,000** sampled rows, with deliberately strict shapes: numerics with
leading zeros stay text, dates must be exactly `YYYY-MM-DD`, booleans must
be exactly `true`/`false`, timestamps must be RFC 3339 or naive ISO, a
column with no sampled non-empty value is text. Inference is a label for
the preview only — **no data is converted** at staging time.

`saya open <file> --typed` builds a second table `<table>_typed` by explicit
`TRY_CAST` per inferred non-text column (`BIGINT`, `DOUBLE`, `BOOLEAN`,
`DATE`, `TIMESTAMP`), on a separate read-write connection to the staged
file — never the session's read-only profile — inside a transaction, leaving
the VARCHAR table untouched. Cast failures are counted against the VARCHAR
table *before* the copy; a value that fails lands NULL in the typed table
and is reported per column ("1 of 1001 values did not cast … (kept NULL)").

### 5. Parquet: metadata caps before decode, in a locked staging connection

The DuckDB `parquet` feature (maintainer-approved) statically links Parquet
support so staging needs no extension `INSTALL`/`LOAD` and no autoload. The
once-read bytes are first written to a private 0600 copy *inside the
destination directory* — the staging connection is pointed only at that
copy, never at the original path (proven by deleting the original after the
read; staging still succeeds).

The staging connection is in-memory and locked down
([`parquet_stage.rs`](../crates/saya-harness/src/file_source/parquet_stage.rs)):
`memory_limit` 256 MB, `threads` 2, autoload and community extensions off,
persistent secrets off, `lock_configuration` — and, unlike every other
saya-owned DuckDB connection, `enable_external_access` **on**, because
reading the private copy and `ATTACH`ing the destination database are
exactly the file operations this connection exists to perform. It is a
throwaway: only fixed SQL ever runs on it, the file path arrives as a bound
parameter, and the connection is dropped before staging returns — user or
model SQL never reaches it.

Before any row is decoded, the caps are checked against metadata: the row
total comes from `parquet_file_metadata(?)` (at most 500,000 rows) and the
column set from a `DESCRIBE SELECT * FROM read_parquet(?)` footer scan (at
most 512 columns) — which also refuses nested types explicitly (`STRUCT`,
`MAP`, `UNION`, list) — only flat columns can be staged. The source's 32 MiB
cap was already enforced at the contained read. Then `CREATE TABLE … AS
SELECT * FROM read_parquet(?) LIMIT <cap+1>` runs inside a transaction on
the attached destination; a count check catches overflow, and a watchdog
thread interrupts any statement still running at the **30-second** wall. A
timeout, cancel, or failure rolls back and deletes the temp database — no
partial table exists. The published file is checkpointed, detached, chmodded
0600, and renamed into place; on success only `source.duckdb` remains.

### 6. Snapshot lifecycle: content-and-contract-addressed, saya-owned only

A snapshot's identity is its content hash **and** its parse contract: the
same file content (same full sha256) **reuses** the existing snapshot
untouched only when the effective delimiter, header flag, table name, and
`--typed` flag also match what the open asks for; otherwise the same content
stages a second snapshot, under a directory name that appends a short digest
of the contract to the content-hash prefix. The snapshot records its
contract and preview in its metadata, and a reused session is built from
that stored metadata alone — never from the freshly staged receipt.
`--reset` restages, refusing to replace a symlink, a directory that is not a
valid saya snapshot, or a valid snapshot whose stored contract differs. A
snapshot is validated by its metadata (a `<sha16>-<contract digest>` name —
or the legacy bare 16-hex content prefix, never reused — matching its
recorded full sha256, regular non-symlink database file) and anything
failing validation is invisible. `saya open --list` shows
newest-first; `saya open --cleanup [<sha-prefix>|all]` removes only
validated snapshot dirs — an ambiguous prefix is refused naming the
matches, and foreign directories or files under the root survive cleanup.

## Consequences

**Accepted costs.**

- Staging duplicates the data: the source bytes are hashed and read once,
  then a DuckDB copy is written. A 32 MiB file costs up to ~64 MiB of disk,
  and snapshots accumulate until `--cleanup` — content-addressing means
  nothing is overwritten, including stale snapshots of changed files.
- The single-read rule means a file changed on disk after staging is not
  seen until the user re-runs `saya open` (and `--reset` if the content
  hash is unchanged but a fresh copy is wanted). The preview's staged time
  and sha prefix are the honest warning.
- Inference is deliberately conservative: leading-zero identifiers, unusual
  date formats, and mixed columns stay text, and `--typed` casts decimals to
  `DOUBLE` (DuckDB's cast target here), not `DECIMAL`. Correctness over
  cleverness — a wrong guess at staging time would be invisible in the
  schema.
- Two locked-down DuckDB configurations now exist in `saya-harness`
  (CSV staging with external access off; Parquet staging with external
  access on). The Parquet one is the only saya-owned connection with
  external access on, it lives entirely inside the staging module, and the
  session gate (§3) refuses the functions that would let any *session*
  reach files.

**Rejected alternatives.**

- *Letting session SQL call `read_parquet`/`read_csv` on demand.* Turns the
  read-only gate into a file-system and network API. The whole point of
  staging is that the gate can then refuse every reader — the staged
  snapshot is the only data the session can see.
- *Opening the file live through a connector.* A live handle re-reads a file
  that can change mid-session, needs its own timeout/interrupt plumbing per
  backend, and would make "what did this result come from?" unanswerable.
  One read, one hash, one snapshot answers it by construction.
- *Type conversion at CSV staging.* An inference mistake at staging would be
  silently baked into the data. VARCHAR storage plus an explicit opt-in
  `--typed` copy keeps the raw text authoritative and the typed table
  auditable.
- *Decoding Parquet first and capping after.* A 10 GiB Parquet file with
  500,001 rows would be fully decoded before being refused; metadata
  (`parquet_file_metadata`, the footer) answers size and shape without
  touching row data, so the caps bind before decode does.

## Limitations (stated, not solved)

- One file per session: no joining across staged snapshots, and no `--sheet`
  (CSV and Parquet have no sheets).
- Caps are hard refusals, not warnings: >500k rows, >512 columns, >32 MiB,
  nested Parquet types — all refuse with no partial staging.
- `--typed` inference labels come from the first ≤1,000 rows only; a column
  that starts numeric but turns textual later still casts, and the failures
  are counted and reported (values kept NULL) rather than refused.
- Parquet staging is bounded to flat schemas; STRUCT/LIST/MAP/UNION columns
  refuse, with no flattening.
- Cleanup is manual (`--cleanup`); nothing expires snapshots automatically.
- The session's row and time caps are the connector's ordinary ones — a
  staged snapshot is queried like any other read-only database.