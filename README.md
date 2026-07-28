# SAYA CLI

SAYA CLI is an open-source, terminal-native shell for a database-aware SAYA
agent. It is currently **private alpha**: PostgreSQL, MySQL, DuckDB, and Snowflake connection
tests, schema discovery, bounded read-only queries, and streaming configured
Ollama/OpenAI-compatible provider calls are implemented alongside the interactive
shell, redacted sessions, and text/JSON/NDJSON output.

## Quick start

```bash
cargo run -p saya-cli -- --help
cargo run -p saya-cli -- config doctor
printf '/help\n/exit\n' | cargo run -p saya-cli --
```

Running `saya` starts a scrollback-preserving REPL. Use `/help` for commands.
The automation surface is available as `saya ask`, `saya query`, `saya config`,
and `saya connection`. PostgreSQL, MySQL, DuckDB, and Snowflake are live engines
in this alpha.

## Configuration

The canonical files are TOML:

```text
.saya/config.toml
.saya/connections.toml
~/.config/saya/config.toml
~/.config/saya/connections.toml
```

Session files default to the platform user-data directory: `SAYA_SESSION_DIR`
if set, then `$XDG_DATA_HOME/saya/sessions`, `%APPDATA%/saya/sessions`, or
`~/.local/share/saya/sessions`. The override is useful for tests and CI.

Use `--config` and `--connections` for explicit paths. Use `--env-file` to opt
into a dotenv-style file; `.env` is never loaded automatically. Process
environment values override explicit env-file values. Store only secret
references such as `{ env = "SAYA_ANALYTICS_PASSWORD" }`, never passwords or
API keys, in committed files. See [configuration](docs/configuration.md) and
[connections](docs/connections.md).

For a local Ollama setup, use a config file plus an explicit env file:

```toml
# .saya/config.toml
[ai]
provider = "ollama"
model = "qwen2.5-coder:14b"
base_url = "http://localhost:11434"
```

```dotenv
# .env.saya (do not commit)
SAYA_PROVIDER=ollama
SAYA_MODEL=qwen2.5-coder:14b
SAYA_PROVIDER_BASE_URL=http://localhost:11434
```

For an OpenAI-compatible service, use a runtime-only API-key reference:

```toml
[ai]
provider = "openai_compatible"
model = "your-model"
base_url = "https://api.example.test/v1"
api_key = { env = "SAYA_API_KEY" }
```

```dotenv
SAYA_API_KEY=replace-me
```

The connection file remains separate:

```toml
[profiles.analytics]
type = "postgresql"
host = "localhost"
port = 5432
database = "warehouse"
user = "saya_readonly"
password = { env = "SAYA_ANALYTICS_PASSWORD" }
sslmode = "require"
```

MySQL uses the same SecretRef password pattern. Its safe default is
`verify-identity`; use `disable` only for an explicitly local development
server. Supported modes are `disable`, `prefer`, `require`, `verify-ca`, and
`verify-identity`:

```toml
[profiles.mysql]
type = "mysql"
host = "localhost"
port = 3306
database = "warehouse"
user = "saya_readonly"
password = { env = "SAYA_MYSQL_PASSWORD" }
sslmode = "verify-identity"
# ssl_ca = { file = "/etc/ssl/certs/mysql-ca.pem" }
```

Snowflake accounts use an account identifier such as `xy12345` or
`org-account.us-east-1.aws`, not a URL. Key-pair, password, and interactive
browser authentication are supported:

```toml
[profiles.snowflake_keypair]
type = "snowflake"
account = "org-account.us-east-1.aws"
user = "jane"
auth_type = "keypair"
private_key = { file = "/absolute/path/to/rsa_key.p8" }
passphrase = { env = "SAYA_SNOWFLAKE_PASSPHRASE" }
warehouse = "ANALYTICS"
database = "PROD"
schema = "PUBLIC"
role = "ANALYST"

[profiles.snowflake_userpass]
type = "snowflake"
account = "org-account.us-east-1.aws"
user = "jane"
auth_type = "userpass"
password = { env = "SAYA_SNOWFLAKE_PASSWORD" }

[profiles.snowflake_browser]
type = "snowflake"
account = "org-account.us-east-1.aws"
user = "jane"
auth_type = "externalbrowser"
```

File SecretRef paths are literal strings: `~` and environment variables are
not expanded. The current runtime supports `env` and `file` SecretRefs;
`{ keyring = "..." }` is a reserved shape and currently reports unavailable.
For an environment-only profile, use an explicit env file or process
environment with `SAYA_DB_TYPE`, `SAYA_DB_ACCOUNT`, `SAYA_DB_USER`, and
`SAYA_DB_AUTH_TYPE`, plus `SAYA_DB_PRIVATE_KEY` for keypair or
`SAYA_DB_PASSWORD` for userpass. Process environment overrides `--env-file`.
`SAYA_DB_PRIVATE_KEY` is raw PEM content. Because env files are line-oriented
and literal `\n` is not converted to a newline, put keypair PEM in a
connections.toml file SecretRef such as `{ file = "/absolute/path/to/rsa_key.p8" }`,
or provide raw multiline PEM through a process environment that preserves it.
Browser authentication requires an interactive TTY and opens a system browser;
it fails before binding, network, or browser launch with `--non-interactive` or
piped input, and the localhost callback expires after 120 seconds.

For a file-backed DuckDB profile, set `read_only` explicitly. `:memory:` may
omit it. DuckDB external access, extension autoloading, community extensions,
and persistent secrets are locked off by the CLI:

```toml
[profiles.local]
type = "duckdb"
path = "./data/warehouse.duckdb"
read_only = true
```

Run `saya --env-file .env.saya --connections .saya/connections.toml
--approval-mode read-only ask "show revenue"`. The newer provider env names
(`SAYA_PROVIDER`, `SAYA_MODEL`, `SAYA_PROVIDER_BASE_URL`, `SAYA_API_KEY`) have
the same precedence as the established `SAYA_AI_*` aliases.

```bash
saya config doctor
saya config show --resolved --redacted --format json
saya connection test analytics --connections examples/connections.toml
saya connection schema analytics --connections examples/connections.toml
saya --non-interactive connection test snowflake_keypair \
  --connections examples/connections.toml
saya --non-interactive connection schema snowflake_keypair \
  --connections examples/connections.toml
saya --non-interactive --env-file .env.snowflake \
  --profile snowflake_userpass connection test snowflake_userpass
saya --profile snowflake_browser connection test snowflake_browser
saya query --profile analytics --sql "SELECT current_database()"
saya --non-interactive --profile snowflake_keypair query \
  --sql "SELECT CURRENT_DATABASE()"
saya --profile snowflake_browser --approval-mode read-only ask \
  "summarize the selected schema"
saya connection test local --connections examples/connections.toml
saya query --profile local --connections examples/connections.toml --sql "SELECT 1"
saya --profile local --approval-mode read-only ask "summarize the local schema"
```

`--non-interactive` is valid for Snowflake keypair and userpass profiles, but
not for `externalbrowser`, which requires an interactive TTY.

## Privacy and limitations

The intended MVP policy is read-only, bounded queries with cloud row sharing
disabled. PostgreSQL, MySQL, DuckDB, and Snowflake reject parse failures, writes, DDL, transaction/control
statements, and multi-statements before execution. It observes one extra row to
mark truncated results. Schema discovery is auto-allowed; bounded SQL is
auto-approved only with `read-only`, denied with `never`, and explicitly
confirmed per query with `ask`. A non-TTY `ask` request is denied safely.
OpenAI and OpenAI-compatible providers are treated as cloud: when sharing is
disabled, they receive schema metadata but not SQL tools or row data. Ollama is
treated as local for this MVP. `/privacy`, `/model`, `/provider`, and `/connect`
apply to the next interactive prompt; `/include` is explicitly display-only and
multi-profile execution is out of scope.
The database role must itself be read-only, and DuckDB file paths must have
least-privilege filesystem permissions: SQL AST checks cannot prove that an
arbitrary database function is free of side effects.
Resolved config secrets, provider headers, and raw query rows are structurally
excluded from session files. Known credential-shaped text is redacted, but
redaction cannot identify every arbitrary secret—never paste credentials into
prompts. See [SECURITY.md](SECURITY.md).

Unavailable or failed connection/schema operations return `3`, while safety and
query failures return `4`; provider/agent failures return `5`. Non-interactive
mode defaults to `never` approval (schema-only) unless `--approval-mode` is
explicit; interactive sessions default to `ask`. This MVP uses complete-chat
HTTP streams token deltas from Ollama and OpenAI-compatible chat-completions. Text output writes
deltas as they arrive; JSON and NDJSON each write one stable JSON event envelope per delta.
Requests retry retryable connection setup failures, HTTP 429, and 5xx responses only before the
provider yields an event. A body transport failure is surfaced without retry because replaying a
partially consumed response cannot be proved action-free. `Ctrl+C` cancels a one-shot request with
exit code 130; during an interactive request it returns to the `saya>` prompt without persisting
the incomplete turn. Interactive prompts and `--continue`/`--resume` use bounded, redacted
user/assistant history, and `/clear` removes visible and provider context. Session files persist
provider/model/profile/privacy/approval settings and safe tool name/status metadata. Database-derived
assistant turns are persisted locally after redaction but omitted from cloud provider history when
sharing is disabled; v1 files fall back to current runtime settings. Raw tool arguments, tool
responses, credentials, headers, and raw tool-result rows are not persisted or reconstructed as
provider history. A natural-language assistant answer may still contain database values.

## Development

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See [CONTRIBUTING.md](CONTRIBUTING.md). The project is Apache-2.0 licensed.
