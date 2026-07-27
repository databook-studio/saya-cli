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

The current alpha has no live database or AI execution path. That limitation
is intentional and should not be bypassed by adding secrets to examples.
