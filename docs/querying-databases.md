# Querying databases with SAYA CLI

SAYA CLI connects to PostgreSQL, MySQL, DuckDB, and Snowflake and runs bounded,
read-only queries. Each command has one primary execution profile, with support
for connecting additional read-only databases using `--include-profile` or
interactive slash commands.
## 1. Initialise a project

```bash
cargo build --release --locked -p saya-cli
./target/release/saya config init
```
This creates credential-free `.saya/config.toml` and `.saya/connections.toml`
without overwriting existing files. Keep private secrets in a local env file,
such as `.env.postgres`, and add `.env.*` to `.gitignore`. Copy and adapt
[`examples/connections.toml`](../examples/connections.toml) for a full template.
For local Docker database services, use
[`examples/connections.docker.toml`](../examples/connections.docker.toml) with
[`examples/.env.docker.example`](../examples/.env.docker.example).

## 2. Configure named profiles

Put all named connections in `.saya/connections.toml`. Credentials must be
references, never literal values. `file` paths must be absolute: `~` and `$VARS`
are not expanded.
```toml
[profiles.analytics]
type = "postgresql"
host = "postgres.internal.example"
port = 5432
database = "analytics"
user = "saya_readonly"
password = { env = "SAYA_ANALYTICS_PASSWORD" }
sslmode = "verify-full"

[profiles.mysql]
type = "mysql"
host = "mysql.internal.example"
port = 3306
database = "orders"
user = "saya_readonly"
password = { env = "SAYA_MYSQL_PASSWORD" }
sslmode = "verify-identity"
ssl_ca = { file = "/etc/ssl/certs/company-mysql-ca.pem" }

[profiles.local]
type = "duckdb"
path = "./data/analytics.duckdb"
read_only = true

[profiles.snowflake_prod]
type = "snowflake"
account = "org-account.us-east-1.aws"
user = "saya_reader"
auth_type = "keypair"
private_key = { file = "/absolute/path/to/snowflake-reader.p8" }
passphrase = { env = "SAYA_SNOWFLAKE_PASSPHRASE" }
warehouse = "ANALYTICS"
database = "PROD"
schema = "PUBLIC"
role = "SAYA_READONLY"
```
Use a database role that is read-only. SAYA rejects writes, DDL, transactions,
and multiple statements, but cannot prove every database function is harmless.
Prefer PostgreSQL `verify-full` and MySQL `verify-identity`; use `disable` only
for a deliberately local development server.
## 3. Provide secrets explicitly

SAYA never loads `.env` automatically. Pass `--env-file`; precedence is CLI
flags, process environment, explicit env file, project TOML, user TOML, then
defaults. Copy these tracked templates to private files:
```dotenv
# .env.postgres — examples/.env.postgres.example
SAYA_PROFILE=analytics
SAYA_ANALYTICS_PASSWORD=replace-with-a-read-only-password

# .env.mysql — examples/.env.mysql.example
SAYA_PROFILE=mysql
SAYA_MYSQL_PASSWORD=replace-with-a-read-only-password

# .env.snowflake — examples/.env.snowflake-userpass.example
SAYA_PROFILE=snowflake_userpass
SAYA_SNOWFLAKE_PASSWORD=replace-with-a-read-only-password
```
Keep Snowflake private keys in a protected file reference, not a dotenv file:
dotenv values are line-oriented and literal `\\n` is not converted to newlines.
The user/password example matches the `snowflake_userpass` profile in the
connection template. `externalbrowser` requires an interactive TTY, so it
cannot run with `--non-interactive`.
For a one-off or CI-only connection, omit `connections.toml` and set the
`SAYA_DB_*` fields in [`examples/.env.example`](../examples/.env.example), or
use [`examples/.env.duckdb.example`](../examples/.env.duckdb.example). Set
`SAYA_PROFILE=env-only` so commands can name that environment-only profile.

## 4. Validate and query

`doctor` reports resolved paths and selection without exposing secrets. Schema
metadata is cached only after a live connection; `--refresh` requires fresh
metadata instead of using a stale fallback.
```bash
saya --connections .saya/connections.toml --env-file .env.postgres config doctor
saya --connections .saya/connections.toml --env-file .env.postgres \
  connection test analytics
saya --connections .saya/connections.toml --env-file .env.postgres \
  connection schema analytics --refresh
# Non-interactive mode defaults to schema-only: grant bounded read-only queries explicitly.
saya --non-interactive --approval-mode read-only \
  --connections .saya/connections.toml --env-file .env.postgres \
  --profile analytics query --sql \
  'SELECT customer_id, COUNT(*) AS orders FROM public.orders GROUP BY 1 LIMIT 20'
saya --non-interactive --approval-mode read-only \
  --connections .saya/connections.toml --env-file .env.mysql \
  --profile mysql query --file ./queries/recent_orders.sql

saya --non-interactive --approval-mode read-only \
  --connections .saya/connections.toml --profile local \
  query --sql 'SELECT table_name FROM information_schema.tables LIMIT 10'
```
`query` accepts one parsed read-only statement; `--file` avoids placing SQL in
shell history. Use `--format json` or `--format ndjson` in scripts. Results are
row-bounded and marked when truncated. Running `saya` without a subcommand
starts the REPL; use `/connect analytics`, `/schema`, then ask with an
explicit approval mode.

## 5. Querying multiple databases

SAYA supports multi-database navigation by connecting additional read-only databases alongside the primary profile:

- **Global flag**: The `--include-profile <name>` global flag (repeatable) connects additional read-only databases alongside the primary `--profile`. It works with both `saya ask` commands and interactive sessions.
- **Interactive slash commands**: In an interactive session, `/include <profile>` and `/exclude <profile>` add and remove secondary live database connections for subsequent turns.
- **Agent navigation across connections**: When more than one database is connected, the AI agent is informed in its system context of the name and SQL dialect of every connected database. The agent navigates between them by passing an optional `connection` argument to its schema-inspection and query tools. It inspects each database separately and combines the findings in its answer.
- **Default connection**: The primary database is the default target when no `connection` argument is specified.
- **Connection failure handling**: If a secondary database fails to connect, it is skipped while the primary run continues. The primary database connection must succeed or the entire run fails.
- **Not single-query SQL federation**: Multi-database support enables AI agent navigation across distinct live connections. It does **not** execute a single federated SQL query that JOINs across engines in a single statement.

### Examples

**Command-line `ask` with included profiles:**
```bash
saya --profile prod --include-profile staging ask "compare row counts"
```

**Interactive session with `/include` and `/exclude`:**
```bash
saya --profile prod
```
```text
> /include staging
Included secondary database 'staging'.

> Compare row counts between prod and staging tables

> /exclude staging
Removed secondary database 'staging'.
```

### Engine-level cross-database joins

For single-statement SQL joins, same-server or same-account databases can perform joins within a single connection if supported by the engine:

| Situation | Supported approach |
| --- | --- |
| Two schemas in one PostgreSQL database | Query qualified names such as `sales.orders` and `support.tickets`. |
| Two MySQL databases on one server | Use `database.table` when the selected read-only user has access to both. |
| Two Snowflake databases in one account | Use `DATABASE.SCHEMA.TABLE` when the selected role has grants on both. |
| Separate engines or standalone databases | Connect multiple profiles with `--include-profile` or `/include` so the AI agent can inspect and query each database independently, combining the results. |

Same-server MySQL and same-account Snowflake can perform single-statement joins inside the selected connection:
```bash
saya --approval-mode read-only --profile mysql query --sql \
  'SELECT o.customer_id, c.segment FROM orders.orders o JOIN crm.customers c USING (customer_id) LIMIT 100'

saya --approval-mode read-only --profile snowflake_prod query --sql \
  'SELECT * FROM PROD.PUBLIC.ORDERS o JOIN CRM.PUBLIC.CUSTOMERS c USING (CUSTOMER_ID) LIMIT 100'
```

## Safety and troubleshooting

- `saya config show --resolved --redacted` confirms precedence without secrets.
- Connection/schema failures exit `3`; rejected or failed queries exit `4`.
- Use a Snowflake keypair file reference for automation; browser SSO is interactive only.
- Protect CA, private-key, and DuckDB files with least-privilege permissions.
- Never place credentials or credential-bearing URLs in SQL, prompts, TOML, or committed env files.
