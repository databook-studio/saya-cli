# Security policy

SAYA CLI is alpha software. Do not use it with production credentials until
the connector and provider implementations have passed security review.

## Secrets

Connection and provider files must contain references, not values. Supported
reference forms are `env`, `file`, and (when a runtime supplies it) `keyring`.
The CLI never auto-loads `.env`; pass `--env-file` explicitly. Diagnostics use
redacted configuration views. Session files contain bounded, redacted
conversation text, selected-profile names, session settings, and safe tool
metadata; query rows, provider headers, and resolved secrets are not part of
the session schema. Known credential-shaped text is redacted when persisted,
but no heuristic can detect every arbitrary user secret; never paste
credentials into prompts.

Session directories default to the platform user-data path and can be changed
with `SAYA_SESSION_DIR`. They are created with mode `0700` and session files
with mode `0600` on Unix. Treat the directory as sensitive and do not commit
it.

## Reporting

Please do not open a public issue for an unpatched vulnerability. Email the
maintainers listed by the `databook-studio` organization with reproduction
steps, affected version, and impact. Do not include live credentials or raw
customer data.

PostgreSQL, MySQL, SQLite, DuckDB, and Snowflake are supported database paths;
Snowflake live validation remains opt-in. Provider execution is available
through Ollama, OpenAI, OpenAI-compatible gateways, Anthropic, and Gemini; fully
offline agent use is not implemented. Release archives are checksummed, but
signing is an external credential and release-plan gate and is not fabricated by
CI or local packaging.

saya enforces read-only at the **database session level** — PostgreSQL
`default_transaction_read_only`, MySQL `transaction_read_only`, SQLite
`query_only`, and a read-only DuckDB open — in addition to fail-closed,
statement-class SQL/AST filtering. Statement filtering alone cannot prove that
an arbitrary database function is side-effect free, and Snowflake has no
equivalent session switch, so you must still connect with a **least-privilege,
read-only database role**. Use restrictive filesystem permissions for
DuckDB/SQLite file paths; do not bypass these boundaries by adding write
credentials to examples.

## Runs write; interactive sessions do not

`saya run` — headless, autonomous runs — is the first part of the product that
writes anything. What changed, and what did not:

**Unchanged: SQL against a database you registered is still read-only by
construction.** The gate takes no mode and no permit parameter, so there is no
configuration that makes it writable. Every backend still crosses it.

**New writable surfaces, each contained — and today only the first is
reachable from any run:**

- **Workspace files.** A run may write inside *its own run directory* and
  nowhere else. Paths are validated before any filesystem call, symlinks are
  refused at every component, opens are no-follow with a post-open identity
  check, and writes are atomic at `0600` and never executable. The tool is
  absent from a run's tool list unless the run approved `workspace-write`.
- **A run-scoped scratch database** (see
  [ADR 0003](docs/adr-0003-scratch-database.md)). One DuckDB file per run, with
  DuckDB's external access disabled, so it can neither read files nor load
  extensions nor reach the network. It is a different type from every user
  database connector and never enters the connection registry — there is no
  path from a scratch write to a database you registered. Not yet reachable
  from any run: its scope is refused until the wiring lands (see
  `docs/commands.md`).
- **Outbound HTTP.** A run may fetch only destinations it declared and you
  approved: HTTPS only, private and link-local addresses refused, redirects
  re-judged at every hop, and every DNS-resolved address checked before
  connecting. DNS rebinding *after* that check is a known, unmitigated
  residual. Fetched content reaches the model only as escaped, delimited
  untrusted context — never as instructions. Not yet reachable from any
  run: its scope is refused until the wiring lands (see `docs/commands.md`).

**None of this applies to the interactive REPL or `saya ask`**, which have the
same read-only posture they always had.

**An autonomous run is a delegation of authority.** Approve the scopes you
actually want, prefer a corpus copy over production data, and read the plan
before approving it — the run will not ask again per step, which is what makes
a long run usable and also what makes the approval matter.
