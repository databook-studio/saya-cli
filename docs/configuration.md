# Configuration

SAYA uses TOML for non-secret settings and connection profiles. The CLI layer
looks for project files in `.saya/` and user files in the platform config
directory (`$XDG_CONFIG_HOME/saya`, `$APPDATA/saya`, or `~/.config/saya`).
Explicit `--config` and `--connections` paths override discovered files.

Value precedence, highest first:

1. CLI flags
2. process environment
3. values from an explicitly supplied `--env-file`
4. project TOML
5. user TOML
6. built-in defaults

The process environment wins over the explicit env-file. `.env` is not loaded
implicitly. This makes CI and scripts predictable:

```bash
saya --env-file .env.saya config show --resolved --redacted
```

Profile selection is `--profile`, `SAYA_PROFILE`, `default_profile`, a sole
profile, then an error when multiple profiles exist. Environment-only mode is
supported by `SAYA_DB_TYPE`, `SAYA_DB_HOST`, `SAYA_DB_PORT`, `SAYA_DB_NAME`,
`SAYA_DB_USER`, `SAYA_DB_PASSWORD`, and optional `SAYA_DB_SSLMODE`. For MySQL,
`SAYA_DB_SSL_CA` is a SecretRef to PEM content and `SAYA_DB_SSLMODE` accepts
`disable`, `prefer`, `require`, `verify-ca`, or `verify-identity`; the safe
default is `verify-identity`. Password and CA values are retained only as
environment references in the typed profile. Use `disable` only for a local
TLS-disabled development server.

Environment-only DuckDB uses `SAYA_DB_TYPE=duckdb` and `SAYA_DB_PATH`. A
file-backed path must also set `SAYA_DB_READ_ONLY=true` or `false`; `:memory:`
may omit it. `SAYA_DB_READ_ONLY` controls DuckDB file access mode and is
distinct from global `SAYA_READ_ONLY`, which controls the SQL/query policy.
Only the exact strings `true` and `false` are accepted for this setting.

`config doctor` reports paths and selection. `config show --resolved
--redacted` emits only display-safe references and settings. It never resolves
or prints secret values.

The REPL session directory uses `SAYA_SESSION_DIR` first, then
`$XDG_DATA_HOME/saya/sessions`, `%APPDATA%/saya/sessions`, or
`~/.local/share/saya/sessions`. In non-interactive mode, an omitted
`--approval-mode` resolves to `never` (schema-only); interactive mode defaults
to `ask`.

The private alpha connects to PostgreSQL, MySQL, and DuckDB. Snowflake remains
unavailable. Environment and file secret
references are resolved at runtime without serializing or logging their values;
keyring references return an explicit unavailable error. Provider settings may
use either the established `SAYA_AI_PROVIDER`, `SAYA_AI_MODEL`, and
`SAYA_AI_BASE_URL` names or the shorter `SAYA_PROVIDER`, `SAYA_MODEL`, and
`SAYA_PROVIDER_BASE_URL` names. `SAYA_API_KEY` becomes a runtime
`{ env = "SAYA_API_KEY" }` reference and is never serialized. Ollama and
OpenAI-compatible chat-completions are supported; other providers are not
implemented yet.

Schema discovery is automatically allowed. A bounded SQL tool call is allowed
under `read-only`, denied under `never`, and asks for explicit `y/yes` on a TTY
under `ask`; it is denied when a TTY is unavailable. Model responses are streamed
for Ollama and OpenAI-compatible providers. Session files persist redacted
user/assistant turn text and safe tool name/status metadata, but omit tool
payloads, credentials, headers, and raw tool-result rows. A database-derived
assistant turn may still contain values in its natural-language answer and is
persisted locally after redaction; it is omitted from cloud provider history
when sharing is disabled. OpenAI and OpenAI-compatible providers are treated
as cloud: with sharing disabled, schema metadata may be sent but the SQL tool
is hidden and dispatcher-blocked, so rows cannot reach those providers. Ollama
is treated as local in this MVP. Interactive `/privacy`, `/model`, `/provider`,
and `/connect` overrides apply to the next prompt; `/include` is not yet a
multi-profile execution feature.
Interactive prompts and resumed sessions reconstruct only bounded, redacted
user/assistant provider history. `/clear` clears visible, persisted, and
provider context. Raw tool arguments, tool responses, and raw tool-result rows
are never restored into provider history. Local Ollama history may include
database-derived turns; v1 sessions without settings use the current runtime
defaults.
