# SAYA CLI

SAYA CLI is an open-source, terminal-native shell for a future database-aware
SAYA agent. It is currently **alpha**: the interactive shell, slash commands,
configuration discovery, redacted sessions, and text/JSON/NDJSON event formats
are implemented. AI provider calls and database connectors are not implemented
in this slice and report honest structured `not_implemented` outcomes.

## Quick start

```bash
cargo run -p saya-cli -- --help
cargo run -p saya-cli -- config doctor
printf '/help\n/exit\n' | cargo run -p saya-cli --
```

Running `saya` starts a scrollback-preserving REPL. Use `/help` for commands.
The automation surface is available as `saya ask`, `saya query`, `saya config`,
and `saya connection`, but live execution is not yet available.

## Configuration

The canonical files are TOML:

```text
.saya/config.toml
.saya/connections.toml
~/.config/saya/config.toml
~/.config/saya/connections.toml
```

Use `--config` and `--connections` for explicit paths. Use `--env-file` to opt
into a dotenv-style file; `.env` is never loaded automatically. Process
environment values override explicit env-file values. Store only secret
references such as `{ env = "SAYA_ANALYTICS_PASSWORD" }`, never passwords or
API keys, in committed files. See [configuration](docs/configuration.md) and
[connections](docs/connections.md).

```bash
saya config doctor
saya config show --resolved --redacted --format json
saya connection list --connections examples/connections.toml
saya --env-file .env.saya --profile analytics
```

## Privacy and limitations

The intended MVP policy is read-only, bounded queries with cloud row sharing
disabled. This alpha does not execute queries or send prompts to providers yet.
No plaintext secrets, provider headers, or raw query rows are persisted in
session files. See [SECURITY.md](SECURITY.md).

## Development

```bash
cargo fmt --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

See [CONTRIBUTING.md](CONTRIBUTING.md). The project is Apache-2.0 licensed.
