use futures_util::TryStreamExt;
use saya_types::{BoundParam, ConnectionError, QueryRequest, QueryResult, SqlDialect};
use serde_json::Value;
use sqlx::{Column as _, Postgres, Row, postgres::PgArguments, query::Query};
use tokio::time::timeout;

use super::{PostgresConnector, decode::json_value, errors};
use crate::binds::BindValue;

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;

pub(crate) async fn query(
    connector: &PostgresConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let _in_flight = connector.in_flight.lock().await;
    let (sql, binds) = prepare(&request.sql, request.max_rows, &request.params)?;
    let mut connection = timeout(connector.query_timeout, connector.pool.acquire())
        .await
        .map_err(|_| ConnectionError::connection_failed("PostgreSQL connection timed out"))?
        .map_err(errors::connection)?;
    let pid = timeout(
        connector.query_timeout,
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection),
    )
    .await
    .map_err(|_| ConnectionError::query_failed("PostgreSQL query timed out"))?
    .map_err(errors::query)?;
    *connector.active_pid.lock().await = Some(pid);
    let result = collect(
        connector,
        &mut connection,
        &sql,
        request.max_rows,
        request.sql,
        &binds,
    )
    .await;
    *connector.active_pid.lock().await = None;
    result
}

/// Prepares the statement for execution. Parameter-free SQL keeps today's
/// prepare path byte-for-byte; parameterized SQL is rewritten to `$n`
/// markers with an ordered bind list — values never touch the text.
fn prepare(
    sql: &str,
    max_rows: usize,
    params: &[BoundParam],
) -> Result<(String, Vec<BindValue>), ConnectionError> {
    if params.is_empty() {
        return Ok((crate::prepare_postgres_sql(sql, max_rows)?, Vec::new()));
    }
    let prepared = crate::prepare_with_params(sql, max_rows, SqlDialect::Postgres, params)?;
    let binds = crate::binds::parse_bind_values(&prepared.values)?;
    Ok((prepared.sql, binds))
}

/// Attaches the ordered bind list to the statement through sqlx's encode
/// path. A null binds as its declared parameter type (B1f): the concrete
/// `Option<T>` per type, so the server sees the declared parameter type —
/// `int8`, `numeric`, `bool`, `date`, `timestamptz`, `text` — instead of
/// inferring one from context (an `IS NULL` test sees it everywhere).
fn bind_query<'q>(
    query: Query<'q, Postgres, PgArguments>,
    values: &'q [BindValue],
) -> Query<'q, Postgres, PgArguments> {
    let mut query = query;
    for value in values {
        query = match value {
            BindValue::Null(param_type) => query.bind(super::TypedNull(*param_type)),
            BindValue::Str(text) => query.bind(text.as_str()),
            BindValue::Int(int) => query.bind(*int),
            BindValue::Bool(flag) => query.bind(*flag),
            BindValue::Decimal { value: decimal, .. } => query.bind(decimal),
            BindValue::Date(date) => query.bind(*date),
            BindValue::Timestamp { value: instant, .. } => query.bind(*instant),
        };
    }
    query
}

async fn collect(
    connector: &PostgresConnector,
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    sql: &str,
    max_rows: usize,
    original_sql: String,
    binds: &[BindValue],
) -> Result<QueryResult, ConnectionError> {
    let work = async {
        let mut stream = bind_query(sqlx::query(sql), binds).fetch(&mut **connection);
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        let mut result_bytes = 0;
        let mut truncated = false;
        while let Some(row) = stream.try_next().await? {
            if columns.is_empty() {
                columns = row
                    .columns()
                    .iter()
                    .map(|column| column.name().into())
                    .collect();
            }
            if rows.len() == max_rows {
                truncated = true;
                break;
            }
            let mut row_values = Vec::with_capacity(row.len());
            for index in 0..row.len() {
                let cell = json_value(&row, index)?;
                let cell = crate::common::cap_cell(cell);
                result_bytes += crate::common::value_bytes(&cell);
                row_values.push(cell);
            }
            rows.push(Value::Array(row_values));

            if result_bytes > crate::common::MAX_RESULT_BYTES {
                truncated = true;
                break;
            }
        }
        Ok(QueryResult {
            row_count: rows.len(),
            columns,
            rows,
            truncated,
            executed_sql: original_sql,
        })
    };
    timeout(connector.query_timeout, work)
        .await
        .map_err(|_| ConnectionError::query_failed("PostgreSQL query timed out"))?
        .map_err(errors::query)
}
