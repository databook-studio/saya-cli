# Connection profiles

Put profiles in `connections.toml` under `[profiles.<name>]`. Values that are
credentials must be references:

```bash
saya config init
```

This creates `.saya/config.toml` and `.saya/connections.toml` only when neither
exists. It is safe to rerun after editing because it refuses to overwrite, and
it rolls back the first file if the second cannot be created. The generated
analytics profile is a PostgreSQL SecretRef template; set its referenced
environment variable before connecting.

```toml
[profiles.analytics]
type = "postgresql"
host = "db.example.test"
port = 5432
database = "warehouse"
user = "saya_readonly"
password = { env = "SAYA_ANALYTICS_PASSWORD" }
sslmode = "require"
```

The private alpha supports `postgresql`, `mysql`, `duckdb`, and `snowflake`.
PostgreSQL supports `disable`, `prefer`, `require`, `verify-ca`,
and `verify-full`; MySQL supports `disable`, `prefer`, `require`, `verify-ca`,
and `verify-identity`.

MySQL defaults to `verify-identity` when `sslmode` is omitted. Passwords and CA
certificates use SecretRefs. An environment CA reference contains PEM content;
use a `{ file = "/path/to/ca.pem" }` reference when the CA is stored on disk.
Use `sslmode = "disable"` only for an explicitly local-only TLS-disabled
development server, never as a production default.

```toml
[profiles.mysql]
type = "mysql"
host = "localhost"
port = 3306
database = "warehouse"
user = "saya_readonly"
password = { env = "SAYA_MYSQL_PASSWORD" }
sslmode = "verify-identity"
ssl_ca = { file = "/etc/ssl/certs/mysql-ca.pem" }
```

DuckDB needs no network credential. File-backed profiles must declare
`read_only`; `:memory:` profiles may omit it. External access, extension
autoloading, community extensions, and persistent secrets are disabled and
locked by the connector. Treat the database path as a filesystem capability:
grant SAYA access only to the intended file and its parent directory.

```toml
[profiles.local]
type = "duckdb"
path = "./warehouse.duckdb"
read_only = true
```

Snowflake profiles require `account`, `user`, and `auth_type`. Account values
are identifiers such as `xy12345` or `org-account.us-east-1.aws`, not URLs.
The auth-specific secret is required for `keypair` and `userpass`; browser SSO
requires no secret:

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

Secret values must not be inline in TOML or committed. File SecretRef paths
are literal; `~` and `$VARS` are not expanded. `env` and `file` references are
supported. A `{ keyring = "..." }` reference is reserved but currently
unavailable in this runtime. For explicit env files, set `SAYA_DB_TYPE`,
`SAYA_DB_ACCOUNT`, `SAYA_DB_USER`, and `SAYA_DB_AUTH_TYPE`, plus
`SAYA_DB_PRIVATE_KEY` for keypair or `SAYA_DB_PASSWORD` for userpass. These
environment values become `SecretRef::Env` references and are not retained in
the typed profile; process environment wins over `--env-file`.
`SAYA_DB_PRIVATE_KEY` is raw PEM content; the line-oriented env-file parser does
not turn literal `\n` into newlines. Prefer a connections.toml file SecretRef
such as `{ file = "/absolute/path/to/rsa_key.p8" }`, or provide raw multiline
PEM through a process environment that preserves newlines.

`externalbrowser` starts a 127.0.0.1 ephemeral callback and exchanges the
browser result for an in-memory legacy session token. It requires a real
interactive TTY; `--non-interactive` and piped input fail before binding,
network, or browser launch. The browser flow has a 120-second overall timeout.

Use these commands to test, inspect, query, or ask against a selected profile:

```bash
saya connection test analytics --connections examples/connections.toml
saya connection schema analytics --connections examples/connections.toml
saya query --profile analytics --sql "SELECT current_database()"
saya --profile local --approval-mode read-only ask "summarize the local schema"
```

Snowflake entry paths are:

```bash
# Keypair from connections.toml (file SecretRef; safe for automation).
saya --non-interactive connection test snowflake_keypair \
  --connections ./connections.toml
saya --non-interactive connection schema snowflake_keypair \
  --connections ./connections.toml

# User/password from an explicit env file (never commit this file).
saya --non-interactive --env-file ./.env.snowflake \
  --profile snowflake_userpass connection test snowflake_userpass

# Browser SSO must run from an interactive TTY.
saya --profile snowflake_browser connection test snowflake_browser
saya --profile snowflake_browser connection schema snowflake_browser

# Bounded SQL and an agent question use the same selected profile.
saya --non-interactive --profile snowflake_keypair query \
  --sql "SELECT CURRENT_DATABASE()"
saya --profile snowflake_browser --approval-mode read-only ask \
  "summarize the selected schema"
```

`--non-interactive` is valid for keypair and userpass profiles. It is not valid
for `externalbrowser`; non-interactive or piped input fails before the browser,
localhost callback, or Snowflake network request is started.

Only one execution profile is selected for a command. `--include-profile` is a
display/session inclusion feature in this alpha, not multi-profile execution.
Anthropic and Gemini are provider-level unavailable features, and fully offline
agent use is unavailable even when the database connector is local.

All four live engines use the same command surface. `query` permits one parsed
read-only statement, caps returned rows, and reports truncation.
Never put
a raw password, private key, API key, or connection URL with embedded
credentials in a committed file. Grant SAYA a database role that is itself
read-only: AST validation cannot establish whether an arbitrary database
function has side effects.
