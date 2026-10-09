mod guard;
#[cfg(test)]
mod tests;

use saya_types::{ConnectionError, SqlDialect};
use sqlparser::{
    ast::{Query, Statement},
    dialect::{
        BigQueryDialect, ClickHouseDialect, Dialect, DuckDbDialect, MySqlDialect,
        PostgreSqlDialect, SQLiteDialect, SnowflakeDialect,
    },
    parser::Parser,
};

use super::read_only_policy::{
    BIGQUERY_POLICY, BackendPolicy, CLICKHOUSE_POLICY, DUCKDB_POLICY, MYSQL_POLICY,
    POSTGRES_POLICY, SNOWFLAKE_POLICY, SQLITE_POLICY,
};
use super::reject::{Rejection, rejected};

use self::guard::guard_statement;

/// The single place that maps a [`SqlDialect`] to the `sqlparser` dialect the
/// safety layer parses with. Shared by the read-only `prepare_*` functions,
/// by object/column extraction, and by the fan-out probe builder so all three
/// parse with the same dialect and never drift apart.
pub(crate) fn parser_dialect(dialect: SqlDialect) -> &'static dyn Dialect {
    match dialect {
        SqlDialect::Postgres => &PostgreSqlDialect {},
        SqlDialect::Mysql => &MySqlDialect {},
        SqlDialect::DuckDb => &DuckDbDialect,
        SqlDialect::Snowflake => &SnowflakeDialect,
        SqlDialect::Sqlite => &SQLiteDialect {},
        SqlDialect::ClickHouse => &ClickHouseDialect {},
        SqlDialect::BigQuery => &BigQueryDialect,
        // `SqlDialect` is `#[non_exhaustive]`. A dialect added later must be
        // wired in explicitly; until then parse as Postgres (the broadest of the
        // five) so the safety layer still rejects or accepts based on syntax.
        _ => &PostgreSqlDialect {},
    }
}

pub fn prepare_postgres_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Postgres),
        &POSTGRES_POLICY,
    )
}

pub fn prepare_mysql_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Mysql),
        &MYSQL_POLICY,
    )
}

pub fn prepare_duckdb_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::DuckDb),
        &DUCKDB_POLICY,
    )
}

pub fn prepare_snowflake_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Snowflake),
        &SNOWFLAKE_POLICY,
    )
}

pub fn prepare_sqlite_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::Sqlite),
        &SQLITE_POLICY,
    )
}

pub fn prepare_clickhouse_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::ClickHouse),
        &CLICKHOUSE_POLICY,
    )
}

pub fn prepare_bigquery_sql(sql: &str, max_rows: usize) -> Result<String, ConnectionError> {
    prepare(
        sql,
        max_rows,
        parser_dialect(SqlDialect::BigQuery),
        &BIGQUERY_POLICY,
    )
}

fn prepare(
    sql: &str,
    max_rows: usize,
    dialect: &dyn Dialect,
    policy: &BackendPolicy,
) -> Result<String, ConnectionError> {
    Ok(parse_guarded(sql, max_rows, dialect, policy)?.to_string())
}

/// Parses one statement under `dialect` — the shared front half of every
/// safety entry.
pub(super) fn parse_single_statement(
    sql: &str,
    dialect: &dyn Dialect,
) -> Result<Statement, ConnectionError> {
    let mut statements = Parser::parse_sql(dialect, sql).map_err(|_| rejected(Rejection::Parse))?;
    if statements.len() != 1 {
        return Err(rejected(Rejection::MultipleStatements));
    }
    Ok(statements.remove(0))
}

/// The shared read-only pipeline: one statement, the guard visitor, the
/// allow-list, the FORMAT check, and the row cap. `prepare` and the
/// parameter-aware preparation in [`super::params`] both run this, so
/// parameters can never fork the policy.
pub(super) fn parse_guarded(
    sql: &str,
    max_rows: usize,
    dialect: &dyn Dialect,
    policy: &BackendPolicy,
) -> Result<Statement, ConnectionError> {
    if max_rows == 0 {
        return Err(rejected(Rejection::RowCap));
    }
    let mut statement = parse_single_statement(sql, dialect)?;
    guard_statement(&mut statement, policy)?;
    if let Some(query) = statement_query(&mut statement) {
        if policy.deny_format_clause && query.format_clause.is_some() {
            return Err(rejected(Rejection::FormatClause));
        }
        cap(query, max_rows);
    }
    Ok(statement)
}

/// The query a statement executes, for row-cap injection. `allowed()` has
/// already narrowed this to `Query` and `Explain(Query)`; capping the inner
/// query of `EXPLAIN ANALYZE` matters because that form *executes* the plan.
fn statement_query(statement: &mut Statement) -> Option<&mut Query> {
    match statement {
        Statement::Query(query) => Some(query),
        Statement::Explain { statement, .. } => match statement.as_mut() {
            Statement::Query(query) => Some(query),
            _ => None,
        },
        _ => None,
    }
}

fn cap(query: &mut Query, max_rows: usize) {
    let limit = literal(query.limit.as_ref());
    let fetch = query
        .fetch
        .as_ref()
        .and_then(|fetch| literal(fetch.quantity.as_ref()));
    // An explicit bound at or under the cap is left exactly as written,
    // including `FETCH FIRST … ROWS ONLY` (Postgres rejects LIMIT+FETCH).
    if limit.or(fetch).is_some_and(|bound| bound <= max_rows) {
        return;
    }
    query.limit = Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(
        max_rows.saturating_add(1).to_string(),
        false,
    )));
    // The injected LIMIT replaces any looser FETCH clause rather than
    // combining with it.
    query.fetch = None;
}

fn literal(limit: Option<&sqlparser::ast::Expr>) -> Option<usize> {
    match limit {
        Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(value, _))) => {
            value.parse().ok()
        }
        _ => None,
    }
}
