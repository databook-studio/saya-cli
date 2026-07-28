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
`SAYA_DB_USER`, `SAYA_DB_PASSWORD`, and optional `SAYA_DB_SSLMODE`; the password
is retained only as an environment reference in the typed profile. PostgreSQL
SSL modes are `disable`, `prefer`, `require`, `verify-ca`, and `verify-full`.

`config doctor` reports paths and selection. `config show --resolved
--redacted` emits only display-safe references and settings. It never resolves
or prints secret values.

The REPL session directory uses `SAYA_SESSION_DIR` first, then
`$XDG_DATA_HOME/saya/sessions`, `%APPDATA%/saya/sessions`, or
`~/.local/share/saya/sessions`. In non-interactive mode, an omitted
`--approval-mode` resolves to `never` (schema-only); interactive mode defaults
to `ask`.

The current alpha connects only to PostgreSQL. Environment and file secret
references are resolved at runtime without serializing or logging their values;
keyring references return an explicit unavailable error. Provider settings may
use either the established `SAYA_AI_PROVIDER`, `SAYA_AI_MODEL`, and
`SAYA_AI_BASE_URL` names or the shorter `SAYA_PROVIDER`, `SAYA_MODEL`, and
`SAYA_PROVIDER_BASE_URL` names. `SAYA_API_KEY` becomes a runtime
`{ env = "SAYA_API_KEY" }` reference and is never serialized. Ollama and
OpenAI-compatible chat-completions are supported; other providers and
non-PostgreSQL engines are not implemented yet.

Schema discovery is automatically allowed. A bounded SQL tool call is allowed
under `read-only`, denied under `never`, and asks for explicit `y/yes` on a TTY
under `ask`; it is denied when a TTY is unavailable. Model responses are not
token-streamed in this MVP. Session files omit tool payloads and assistant
database output by default. OpenAI and OpenAI-compatible providers are treated
as cloud: with sharing disabled, schema metadata may be sent but the SQL tool
is hidden and dispatcher-blocked, so rows cannot reach those providers. Ollama
is treated as local in this MVP. Interactive `/privacy`, `/model`, `/provider`,
and `/connect` overrides apply to the next prompt; `/include` is not yet a
multi-profile execution feature.
