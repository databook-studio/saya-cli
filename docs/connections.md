# Connection profiles

Put profiles in `connections.toml` under `[profiles.<name>]`. Values that are
credentials must be references:

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

The private alpha supports `postgresql`, `mysql`, and `duckdb`. Snowflake
configuration is accepted for forward compatibility but its connector remains
unavailable. PostgreSQL supports `disable`, `prefer`, `require`, `verify-ca`,
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

Use these commands to test, inspect, query, or ask against a selected profile:

```bash
saya connection test analytics --connections examples/connections.toml
saya connection schema analytics --connections examples/connections.toml
saya query --profile analytics --sql "SELECT current_database()"
saya --profile local --approval-mode read-only ask "summarize the local schema"
```

All three live engines use the same command surface. `query` permits one parsed
read-only statement, caps returned rows, and reports truncation.
Never put
a raw password, private key, API key, or connection URL with embedded
credentials in a committed file. Grant SAYA a database role that is itself
read-only: AST validation cannot establish whether an arbitrary database
function has side effects.
