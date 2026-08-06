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

PostgreSQL, MySQL, DuckDB, and Snowflake are supported database paths;
Snowflake live validation remains opt-in. Provider execution
exists through the supported Ollama/OpenAI-compatible interfaces only;
Anthropic, Gemini, and fully offline agent use are unavailable. Release
archives are checksummed, but signing is an external credential and release
plan gate and is not fabricated by CI or local packaging. The SQL
policy is deliberately fail-closed, but it cannot prove arbitrary database
functions are side-effect free. Use least-privilege, read-only database
credentials and restrictive filesystem permissions for DuckDB paths; do not
bypass those boundaries by adding write credentials to examples.
