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

`postgresql` is the only implemented engine. It supports `disable`, `prefer`,
`require`, `verify-ca`, and `verify-full` `sslmode` values. The `mysql`,
`duckdb`, and `snowflake` profile shapes are retained for forward compatibility,
but report structured unavailable outcomes; Snowflake configuration accepts
`keypair`, `userpass`, and `externalbrowser` auth types.

DuckDB needs no network credential:

```toml
[profiles.local]
type = "duckdb"
path = "./warehouse.duckdb"
read_only = true
```

Use `saya connection test analytics` to validate PostgreSQL credentials and
`saya connection schema analytics` to discover schemas, tables, and columns.
`saya query --profile analytics --sql "SELECT ..."` permits one parsed
read-only statement only, caps returned rows, and reports truncation. Never put
a raw password, private key, API key, or connection URL with embedded
credentials in a committed file.
