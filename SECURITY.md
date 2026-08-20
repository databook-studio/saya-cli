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
