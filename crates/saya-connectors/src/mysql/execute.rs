use futures_util::TryStreamExt;
use saya_types::{BoundParam, ConnectionError, QueryRequest, QueryResult, SqlDialect};
use serde_json::Value;
use sqlx::{Column as _, MySql, Row, mysql::MySqlArguments, query::Query};
use tokio::time::timeout;

use super::{MySqlConnector, decode::json_value, errors};
use crate::binds::BindValue;

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;

pub(crate) async fn query(
    connector: &MySqlConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let (sql, binds) = prepare(&request.sql, request.max_rows, &request.params)?;
    // Serialize executes so `active_id` is unambiguous for cancellation.
    let _in_flight = connector.in_flight.lock().await;
    // Defect B fix: acquire ONE pooled connection, capture its id on it, and
    // run the query on the same connection. Letting the pool hand out a
    // different connection between `CONNECTION_ID()` and the query made the
    // recorded id belong to the wrong session, so `KILL QUERY` would target
    // an idle connection and leave the runaway query running.
    let mut connection = timeout(connector.query_timeout, connector.pool.acquire())
        .await
        .map_err(|_| ConnectionError::connection_failed("MySQL connection timed out"))?
        .map_err(errors::connection)?;
    let id: u64 = timeout(
        connector.query_timeout,
        sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&mut *connection),
    )
    .await
    .map_err(|_| ConnectionError::connection_failed("MySQL connection timed out"))?
    .map_err(errors::connection)?;
    *connector.active_id.lock().await = Some(id);
    // The timeout stays here rather than inside `collect` so that a timeout is
    // distinguishable from a query that failed on its own.
    //
    // Defect A fix: do NOT clear `active_id` before cancelling. On timeout we
    // hand the id to `cancel`, which claims it (single-winner `take`) and
    // issues the kill before we return. Clearing first — the old code — made
    // the kill read `None` and become a no-op, so a timed-out query kept
    // running server-side.
    //
    // Only a timeout earns a kill. A query that failed on its own — bad SQL, a
    // missing table — has already finished server-side, and killing it would
    // open a fresh connection to `KILL QUERY` an id that is no longer running.
    // Invalid SQL is routine in an agent loop, so that cost would be paid
    // constantly for nothing.
    let Ok(result) = timeout(
        connector.query_timeout,
        collect(&mut connection, &sql, request.max_rows, request.sql, &binds),
    )
    .await
    else {
        super::cancellation::cancel(connector).await.ok();
        return Err(ConnectionError::query_failed("MySQL query timed out"));
    };
    *connector.active_id.lock().await = None;
    result
}

/// Prepares the statement for execution. Parameter-free SQL keeps today's
/// prepare path byte-for-byte; parameterized SQL is rewritten to `?` markers
/// with an ordered bind list — values never touch the text.
fn prepare(
    sql: &str,
    max_rows: usize,
    params: &[BoundParam],
) -> Result<(String, Vec<BindValue>), ConnectionError> {
    if params.is_empty() {
        return Ok((crate::prepare_mysql_sql(sql, max_rows)?, Vec::new()));
    }
    let prepared = crate::prepare_with_params(sql, max_rows, SqlDialect::Mysql, params)?;
    let binds = crate::binds::parse_bind_values(&prepared.values)?;
    Ok((prepared.sql, binds))
}

/// Attaches the ordered bind list to the statement through sqlx's encode
/// path. A timestamp binds as the instant's UTC wall clock (`NaiveDateTime`):
/// sqlx-mysql has no `DateTime<FixedOffset>` encode, and `MYSQL_TYPE_DATETIME`
/// compares literally against `DATETIME` columns with no session-time-zone
/// conversion. A null binds as `Option::<String>::None` — MySQL types NULL
/// parameters from context.
fn bind_query<'q>(
    query: Query<'q, MySql, MySqlArguments>,
    values: &'q [BindValue],
) -> Query<'q, MySql, MySqlArguments> {
    let mut query = query;
    for value in values {
        query = match value {
            BindValue::Null => query.bind(Option::<String>::None),
            BindValue::Str(text) => query.bind(text.as_str()),
            BindValue::Int(int) => query.bind(*int),
            BindValue::Bool(flag) => query.bind(*flag),
            BindValue::Decimal { value: decimal, .. } => query.bind(decimal),
            BindValue::Date(date) => query.bind(*date),
            BindValue::Timestamp { value: instant, .. } => query.bind(instant.naive_utc()),
        };
    }
    query
}

/// Streams one already-prepared statement into a bounded `QueryResult`. The
/// caller owns the timeout, so it can tell a timeout apart from a query that
/// failed on its own — only the former earns a `KILL QUERY`.
async fn collect(
    connection: &mut sqlx::pool::PoolConnection<sqlx::MySql>,
    sql: &str,
    max_rows: usize,
    original_sql: String,
    binds: &[BindValue],
) -> Result<QueryResult, ConnectionError> {
    let mut stream = bind_query(sqlx::query(sql), binds).fetch(&mut **connection);
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    let mut result_bytes = 0;
    let mut truncated = false;
    while let Some(row) = stream.try_next().await.map_err(errors::query)? {
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
            let cell = json_value(&row, index).map_err(errors::query)?;
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
}
