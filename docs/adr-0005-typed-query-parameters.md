# ADR 0005: Typed runtime query parameters for saved investigations

- Status: accepted 2026-09-29. Records what shipped for release 0.4.2's
  typed-parameter milestone (B1), verified against the code; where the plan
  and the code differ, the code is recorded.
- Date: 2026-09-29
- Supersedes: nothing. Complements [ADR 0004](adr-0004-saved-investigations-and-evidence.md)
  (the saved investigation this extends) and [ADR 0002](adr-0002-memory-and-contract-trust-model.md)
  (what never reaches a provider — parameter values are bound to that list).
- Records why saved SQL carries named `:name` placeholders instead of baked-in
  literals, how values bind through the read-only gate without ever being
  substituted into SQL text or persisted anywhere, and which engines accept
  them.

## Context

ADR 0004 made a good answer replayable: the exact SQL, saved as a portable
JSON document, re-run through the same read-only gate. That exactness has a
cost. A question answered for one region, date range, or customer cannot be
replayed for another without editing the SQL — which creates a new revision,
invalidates the review, and pushes the whole review burden back onto the
user. The alternative people reach for is string substitution: splice the
value into the text and save it. That is both an injection hazard and a
semantics change — a value spliced as text parses differently from a bound
one (`'007'`, dates, decimals with trailing digits), and the document would
carry whatever the substituting tool decided to quote.

What is wanted is the ordinary database capability: a prepared statement
whose parameters bind at execute time. The constraints are the ones the
product already lives by: every statement crosses the read-only AST gate
([ADR 0003](adr-0003-scratch-database.md)); nothing about a query's values
reaches a provider, a session file, evidence, or any persisted record
([ADR 0002](adr-0002-memory-and-contract-trust-model.md)); and the saved
document must stay portable across all seven dialects.

## Decision

### 1. The portable syntax is the named placeholder `:name` — nothing else

Saved SQL binds parameters by name, written `:name`, with names matching
`[a-z_][a-z0-9_]{0,31}` ([`is_valid_param_name`](../crates/saya-types/src/params/spec.rs)).
sqlparser 0.53 parses `:name` as `Value::Placeholder` in all seven dialects,
so one spelling survives every backend. Any other placeholder form — `$1`,
`?`, `?1`, `@x`, or ClickHouse's `{name:Type}` — is refused wherever the
placeholders are collected
([`collect_names`](../crates/saya-connectors/src/safety/params/rewrite.rs)):
a positional marker has no portable name to bind by, and sqlparser 0.53
cannot parse the ClickHouse form at all.

Placeholders are AST nodes, not text: a `:region` inside a string literal or
a comment is part of that literal, is never collected, and is never bound
(pinned by `placeholder_in_string_or_comment_is_not_bound`,
[`params_tests.rs`](../crates/saya-connectors/src/safety/params_tests.rs)).

### 2. The document declares what the SQL binds

`InvestigationDefinitionV1` gains `parameters: Vec<ParameterSpec>`
([`investigation/mod.rs`](../crates/saya-types/src/investigation/mod.rs)):
serde-defaulted, omitted when empty, at most 32
([`MAX_PARAMETERS`](../crates/saya-types/src/params/spec.rs)). Each spec is
`{ name, type, required, description? }`; the types are `string`, `integer`,
`boolean`, `decimal`, `date`, and `timestamp`; descriptions are capped at
256 bytes and refuse control characters. There are deliberately no
identifier, list, or SQL-fragment types: a parameter must be a value, or the
gate could be talked into rewriting its own policy.

Declaration and SQL must agree in both directions, and the check is applied
at every point a document can enter the store — save, edit, and import all
run [`check_contract`](../crates/saya-cli/src/commands/investigation/params.rs)
over [`sql_placeholders`](../crates/saya-connectors/src/safety/params.rs), and
at run time the safety layer re-proves the same equality
(`refuse_unbalanced`). `saya-types` validates the spec list but not the SQL
match — it has no SQL parser — so the equality lives with the code that
parses SQL. Changing the parameter list is a document edit, and any edit
publishes a new revision; the local review binding stays on the old
revision, so the next run refuses with `review is stale (revision changed)`
until `--revalidate`, exactly like any other edit.

### 3. Values are typed at the boundary, strictly

`ParamValue`
([`params/value.rs`](../crates/saya-types/src/params/value.rs)) is an enum —
`Null(ParamType)`, `String`, `Integer(i64)`, `Boolean`, `Decimal(String)`,
`Date(String)`, `Timestamp(String)` — where decimal, date, and timestamp
keep their validated text exactly as bound. Parsing
([`parse.rs`](../crates/saya-types/src/params/parse.rs)) is strict: no
trimming, no coercion, no `+` sign, no exponent; decimals allow at most 38
digits; dates are real proleptic-Gregorian (`2024-02-29` parses,
`2023-02-29` does not); timestamps are RFC 3339 with a mandatory offset — a
space separator and a leap second are refused. The literal `null` binds a
typed null carrying the parameter's declared type, which also means a string
value of `null` is not expressible.

`ParamValue`'s `Debug` prints the variant only, never the value, so bindings
survive logs and transcripts without leaking. `QueryRequest` carries
`params: Vec<BoundParam>` (serde-defaulted, built via
[`with_params`](../crates/saya-types/src/query.rs)); the safety layer matches
bound names against the SQL's placeholders exactly, in both directions.

### 4. The safety layer rewrites markers in the AST — it never substitutes text

[`prepare_with_params`](../crates/saya-connectors/src/safety/params.rs) is a
parameter-aware entry beside `prepare`, not a fork of it: the same
`parse_guarded` pipeline runs first — single statement, the read-only guard,
the per-backend allow-list, and the row cap — and only then are placeholders
collected and checked. Refusals name placeholders, never values. The rewrite
writes the dialect's native marker *into the AST node*, which `Display` then
prints verbatim — no string substitution anywhere.

PostgreSQL gets `$n`, numbered by first occurrence, with a repeated name
reusing its number and its value once (`WHERE b = :city AND c > :floor AND
d = :city` becomes `b = $1 AND c > $2 AND d = $1` with two values). The
other five dialects get `?`, with the value repeated per occurrence in AST
order. Both orderings are pinned by
[`params_tests.rs`](../crates/saya-connectors/src/safety/params_tests.rs).
One collision is worth stating: `LIMIT :rows` cannot be parameterised, because
the row-cap injection replaces the limit clause and the binding surfaces as
an unused-parameter refusal — the cap is the policy, not a value.

### 5. Each engine binds natively, and the capability is published

`DatabaseConnector::supports_parameters` defaults to `false`
([`lib.rs`](../crates/saya-connectors/src/lib.rs)); a connector that does not
bind natively refuses a non-empty parameter list before any connection
attempt, while parameter-free SQL works everywhere unchanged. What shipped:

| Dialect | Marker | Native binding | Typed null | Notes |
| --- | --- | --- | --- | --- |
| PostgreSQL | `$n` | sqlx `bind_query` | declared PG type (`TypedNull`: `int8`, `numeric`, `bool`, `date`, `timestamptz`, `text`) | decimals as `BigDecimal`; timestamps keep their offset |
| MySQL | `?` | sqlx `bind_query` | untyped `Option::None` — the server types NULL from context | timestamps bind as the instant's UTC wall clock (`NaiveDateTime`), because `MYSQL_TYPE_DATETIME` compares literally with no session-tz conversion |
| SQLite | `?` | sqlx `bind_query` | untyped `Option::None` | SQLite is dynamically typed: decimals and timestamps bind as the exact validated text, so stored forms like `007` compare as written |
| DuckDB | `?` | native values — `DECIMAL(width, scale)` payload, `Date32`, microseconds | `Value::Null` | decimals are exact to the column's scale; a text bind was probed and **rejected** because DuckDB's VARCHAR→DECIMAL cast rounds to the compared column's scale (`19.99 > 19.988` must be true; a text bind returns zero rows). More than 38 digits refuses |
| Snowflake | `?` | SQL API v2 `bindings` — **keypair auth only** | type `ANY` | values are strings keyed by 1-based position: DATE as epoch milliseconds, TIMESTAMP as epoch nanoseconds with the offset piggybacked. Non-keypair auth refuses before any network activity (`parameters need keypair auth on Snowflake`) |
| BigQuery | `?` | POSITIONAL `queryParameters` — the dry run carries the same list, so the estimate is computed against the statement that will actually run | declared `parameterType` with a JSON null (null decimal → `NUMERIC`) | `NUMERIC` vs `BIGNUMERIC` chosen by digits and scale; timestamps canonicalised to UTC RFC 3339 |
| ClickHouse | — | unsupported | — | refuses before any network activity, with no "yet": sqlparser 0.53 cannot parse `{name:Type}`, and the HTTP interface has no binding the portable pipeline can target. Fixed (parameter-free) SQL still works |

Before any engine sees a value, [`parse_bind_values`](../crates/saya-connectors/src/binds.rs)
re-checks decimal, date, and timestamp text — a deserialised request can
carry any string, so nothing reaches an engine on the strength of an earlier
parse. No error message names a value.

### 6. Values never persist — anywhere

The evidence record carries parameter **names**, in declaration order, plus
`params_sha256` — a SHA-256 over the canonical `name=value` lines of the
bound set in declaration order
([`evidence.rs`](../crates/saya-types/src/evidence.rs)). Never a value. The
definition stores declarations only; audit rows, session files, replay
payloads, and reports carry names or nothing; `Debug` redaction at
`ParamValue`, `BindValue`, and `PreparedQuery` keeps values out of logs. The
property is pinned end-to-end by
[`parameter_values_never_persist`](../crates/saya-cli/tests/investigation_params.rs),
which runs a replay with a sentinel value and then scans every file under
the harness root — definition, binding, state store, config, connections,
session state, report — plus stdout and stderr, asserting the value appears
nowhere.

### 7. Surfaces

`saya investigation save … --param-spec name:type[:required]` (repeatable)
declares parameters; `edit --param-spec` replaces the whole list;
`saya investigation run <id> --param name=value` (repeatable) binds them,
with `null` spelling a typed null and an omitted optional binding a typed
null too. `/investigation run <id> --param …` in the TUI parses into the
same `InvestigationCommand`. Binding errors — unknown name, bad value,
missing required — exit 2 and are checked before any store, profile, or
connection work; a missing required parameter lists every required name with
its type. Replay with parameters stays provider-free: it is the same query
path as `saya query` with bindings attached. The MCP `investigation_run`
tool takes an id and optional profile only — parameter binding over MCP is
pending (see [ADR 0008](adr-0008-mcp-server.md)).

## Consequences

**Accepted costs.**

- A second preparation entry exists beside `prepare`. The two share one
  pipeline (`parse_guarded`), and a test pins that parameter-free SQL passes
  through `prepare_with_params` byte-identically to `prepare`, so they can
  only drift if the shared pipeline changes.
- Typed-null typing is per-engine, because the engines are: PostgreSQL and
  BigQuery bind a declared type, Snowflake sends `ANY`, MySQL and SQLite
  bind an untyped `Option::None` (MySQL types NULL from context). Each
  choice is pinned by that connector's tests rather than papered over.
- The strict value grammar refuses inputs a human would accept ("no
  trimming, no coercion") — the deliberate price of values that compare
  exactly as written everywhere.
- ClickHouse users get an honest fixed-SQL-only experience until sqlparser
  can parse the `{name:Type}` form.

**Rejected alternatives.**

- *String substitution into saved SQL.* The injection hazard is obvious, but
  the subtler failure is semantic: a spliced value parses as text, so
  decimals lose digits and dates gain dialect-specific quoting. Binding
  through the engine's own parameter API is what makes the value mean the
  same thing on every backend.
- *Persisting bound values in the document, binding, or evidence.* The
  document is a shareable artifact; a value baked into it is unreviewed data
  leaving the machine (ADR 0002's line). A digest of the value set is enough
  to correlate evidence runs without carrying the values.
- *Positional parameters (`$1`, `?1`) in saved SQL.* A positional list is an
  ordering contract invisible in the SQL; one reordered `--param` and the
  query silently means something else. Names make the binding auditable
  against the text.
- *Supporting ClickHouse via `{name:Type}`.* sqlparser 0.53 cannot parse the
  form, so "support" would mean a second, non-AST substitution path —
  precisely the thing this ADR exists to refuse.

## Limitations (stated, not solved)

- Parameters are available to saved investigations and the TUI's
  `/investigation run`; direct `saya query` has no parameter flags.
- The MCP `investigation_run` tool cannot bind parameters yet.
- `LIMIT :rows` cannot be parameterised (the row-cap injection replaces the
  limit clause first).
- No list, identifier, or SQL-fragment parameter types — by design.
- Snowflake requires keypair auth for parameters; the legacy/browser path
  refuses, and nothing falls back to it.
- ClickHouse refuses all parameterised SQL; its parameter-free path is
  unchanged.
- A string value of `null` is not expressible — `null` always binds a typed
  null.