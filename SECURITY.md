# Security policy

SAYA CLI is alpha software. Do not use it with production credentials until
the connector and provider implementations have passed security review.

## Secrets

Connection and provider files must contain references, not values. Supported
reference forms are `env`, `file`, and (when a runtime supplies it) `keyring`.
The CLI never auto-loads `.env`; pass `--env-file` explicitly. Diagnostics use
redacted configuration views. Session files contain conversation text and
profile names only; query rows, provider headers, and resolved secrets are not
part of the session schema. Known credential-shaped text is redacted when
persisted, but no heuristic can detect every arbitrary user secret; never paste
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

PostgreSQL, MySQL, and DuckDB are live database paths in the private alpha, and
provider execution exists through the supported Ollama/OpenAI-compatible
interfaces. Snowflake remains unavailable. The SQL policy is deliberately
fail-closed, but it cannot prove arbitrary database functions are side-effect
free. Use least-privilege, read-only database credentials and restrictive
filesystem permissions for DuckDB paths; do not bypass those boundaries by
adding write credentials to examples.
