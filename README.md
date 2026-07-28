# SAYA CLI

SAYA CLI is an open-source, terminal-native shell for a database-aware SAYA
agent. It is currently **alpha**: PostgreSQL connection tests, schema discovery,
bounded read-only queries, and configured Ollama/OpenAI-compatible provider calls
are implemented alongside the interactive shell, redacted sessions, and
text/JSON/NDJSON output. Other database engines remain unavailable.

## Quick start

```bash
cargo run -p saya-cli -- --help
cargo run -p saya-cli -- config doctor
printf '/help\n/exit\n' | cargo run -p saya-cli --
```

Running `saya` starts a scrollback-preserving REPL. Use `/help` for commands.
The automation surface is available as `saya ask`, `saya query`, `saya config`,
and `saya connection`. PostgreSQL is the only live engine in this alpha.

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

Run `saya --env-file .env.saya --connections .saya/connections.toml
--approval-mode read-only ask "show revenue"`. The newer provider env names
(`SAYA_PROVIDER`, `SAYA_MODEL`, `SAYA_PROVIDER_BASE_URL`, `SAYA_API_KEY`) have
the same precedence as the established `SAYA_AI_*` aliases.

```bash
saya config doctor
saya config show --resolved --redacted --format json
saya connection test analytics --connections examples/connections.toml
saya connection schema analytics --connections examples/connections.toml
saya query --profile analytics --sql "SELECT current_database()"
```

## Privacy and limitations

The intended MVP policy is read-only, bounded queries with cloud row sharing
disabled. PostgreSQL rejects parse failures, writes, DDL, transaction/control
statements, and multi-statements before execution. It observes one extra row to
mark truncated results. Schema discovery is auto-allowed; bounded SQL is
auto-approved only with `read-only`, denied with `never`, and explicitly
confirmed per query with `ask`. A non-TTY `ask` request is denied safely.
The database role must itself be read-only: SQL AST checks cannot prove that a
PostgreSQL function is free of side effects.
Resolved config secrets, provider headers, and raw query rows are structurally
excluded from session files. Known credential-shaped text is redacted, but
redaction cannot identify every arbitrary secret—never paste credentials into
prompts. See [SECURITY.md](SECURITY.md).

Unavailable or failed connection/schema operations return `3`, while safety and
query failures return `4`; provider/agent failures return `5`. Non-interactive
mode defaults to `never` approval (schema-only) unless `--approval-mode` is
explicit; interactive sessions default to `ask`. This MVP uses complete-chat
HTTP requests and does not yet stream model tokens.

## Development

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See [CONTRIBUTING.md](CONTRIBUTING.md). The project is Apache-2.0 licensed.
