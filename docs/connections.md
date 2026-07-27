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
```

Supported profile shapes are `postgresql`, `mysql`, `duckdb`, and `snowflake`.
Snowflake accepts `keypair`, `userpass`, and `externalbrowser` auth types in
the typed configuration contract; execution is unfinished in this alpha.

DuckDB needs no network credential:

```toml
[profiles.local]
type = "duckdb"
path = "./warehouse.duckdb"
read_only = true
```

Use `saya connection list` to inspect profile names and dialects. `connection
test` and `connection schema` currently return structured not-implemented
outcomes. Never put a raw password, private key, API key, or connection URL
with embedded credentials in a committed file.
