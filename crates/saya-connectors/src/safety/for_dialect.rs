//! Dialect-generic entry to the execution-time read-only gate.

use saya_types::{ConnectionError, SqlDialect};

use super::read_only::{
    prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_mysql_sql,
    prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql,
};

/// Prepares read-only `sql` for [`SqlDialect`] by dispatching to the same
/// per-dialect `prepare_*` wrapper the connector calls at execution time, so
/// callers holding a dialect value (e.g. saved-investigation save/import) get
/// the exact safety gate without matching on the dialect themselves.
///
/// Unlike some call sites, there is no fallback dialect: a [`SqlDialect`]
/// variant without a wired wrapper is refused.
pub fn prepare_for_dialect(
    sql: &str,
    max_rows: usize,
    dialect: SqlDialect,
) -> Result<String, ConnectionError> {
    match dialect {
        SqlDialect::Postgres => prepare_postgres_sql(sql, max_rows),
        SqlDialect::Mysql => prepare_mysql_sql(sql, max_rows),
        SqlDialect::DuckDb => prepare_duckdb_sql(sql, max_rows),
        SqlDialect::Snowflake => prepare_snowflake_sql(sql, max_rows),
        SqlDialect::Sqlite => prepare_sqlite_sql(sql, max_rows),
        SqlDialect::ClickHouse => prepare_clickhouse_sql(sql, max_rows),
        SqlDialect::BigQuery => prepare_bigquery_sql(sql, max_rows),
        // `SqlDialect` is `#[non_exhaustive]`: a variant added later must be
        // wired in explicitly. Until then fail closed — refusing is safer than
        // silently applying another dialect's policy.
        _ => Err(ConnectionError::unsupported(format!(
            "read-only preparation is not wired for the {dialect:?} dialect"
        ))),
    }
}
