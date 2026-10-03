use std::time::Instant;

use futures_util::TryStreamExt;
use saya_types::{BoundParam, ConnectionError, QueryRequest, QueryResult, SqlDialect};
use serde_json::Value;
use sqlx::{Column as _, Row, Sqlite, query::Query, sqlite::SqliteArguments};
use std::sync::atomic::Ordering;
use tokio::time::timeout;

use super::{SqliteConnector, decode, errors};
use crate::binds::BindValue;

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;

pub(crate) async fn query(
    c: &SqliteConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let (sql, binds) = prepare(&request.sql, request.max_rows, &request.params)?;
    let _attempt = c.begin_attempt();

    let mut conn = timeout(c.query_timeout, c.pool.acquire())
        .await
        .map_err(|_| ConnectionError::query_failed("SQLite query timed out"))?
        .map_err(errors::query)?;
    let deadline = Instant::now() + c.query_timeout;
    let cancelled = c.cancelled.clone();

    {
        let mut handle = conn.lock_handle().await.map_err(errors::query)?;
        // The handler keeps the query going only while inside the deadline and not
        // cancelled. Returning `false` aborts the running statement from inside
        // the SQLite VM — the same mechanism the deadline already used.
        handle.set_progress_handler(1000, move || {
            Instant::now() < deadline && !cancelled.load(Ordering::Acquire)
        });
    }

    let stream_res = fetch_rows(&mut conn, &sql, request.max_rows, &binds).await;

    if let Ok(mut handle) = conn.lock_handle().await {
        handle.remove_progress_handler();
    }

    let (columns, rows, truncated) = match stream_res {
        Ok(res) => res,
        Err(err) => {
            if c.cancelled.load(Ordering::Acquire) {
                return Err(ConnectionError::cancelled());
            }
            if Instant::now() >= deadline || is_interrupt_error(&err) {
                return Err(ConnectionError::query_failed("SQLite query timed out"));
            }
            return Err(errors::query(err));
        }
    };

    Ok(QueryResult {
        row_count: rows.len(),
        columns,
        rows,
        truncated,
        executed_sql: request.sql,
    })
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
        return Ok((crate::prepare_sqlite_sql(sql, max_rows)?, Vec::new()));
    }
    let prepared = crate::prepare_with_params(sql, max_rows, SqlDialect::Sqlite, params)?;
    let binds = crate::binds::parse_bind_values(&prepared.values)?;
    Ok((prepared.sql, binds))
}

/// Attaches the ordered bind list to the statement through sqlx's encode
/// path. SQLite is dynamically typed: decimals and timestamps bind as the
/// exact validated text (so stored forms like `007` or a lowercase `t`
/// compare as written), dates as `NaiveDate` (`%F` text), a null as
/// `Option::<String>::None`.
fn bind_query<'q>(
    query: Query<'q, Sqlite, SqliteArguments<'q>>,
    values: &'q [BindValue],
) -> Query<'q, Sqlite, SqliteArguments<'q>> {
    let mut query = query;
    for value in values {
        query = match value {
            BindValue::Null(_) => query.bind(Option::<String>::None),
            BindValue::Str(text) => query.bind(text.as_str()),
            BindValue::Int(int) => query.bind(*int),
            BindValue::Bool(flag) => query.bind(*flag),
            BindValue::Decimal { text, .. } => query.bind(text.as_str()),
            BindValue::Date(date) => query.bind(*date),
            BindValue::Timestamp { text, .. } => query.bind(text.as_str()),
        };
    }
    query
}

async fn fetch_rows(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    sql: &str,
    max_rows: usize,
    binds: &[BindValue],
) -> Result<(Vec<String>, Vec<Value>, bool), sqlx::Error> {
    let mut stream = bind_query(sqlx::query(sql), binds).fetch(&mut **conn);
    let mut columns = Vec::new();
    let mut rows = Vec::new();
    let mut result_bytes = 0;
    let mut truncated_by_bytes = false;

    while let Some(row) = stream.try_next().await? {
        if columns.is_empty() {
            columns = row
                .columns()
                .iter()
                .map(|column| column.name().to_string())
                .collect();
        }
        let mut row_values = Vec::with_capacity(row.len());
        for index in 0..row.len() {
            let cell = decode::json_value(&row, index)?;
            let cell = crate::common::cap_cell(cell);
            result_bytes += crate::common::value_bytes(&cell);
            row_values.push(cell);
        }
        rows.push(Value::Array(row_values));

        if rows.len() > max_rows {
            break;
        }
        if result_bytes > crate::common::MAX_RESULT_BYTES {
            truncated_by_bytes = true;
            break;
        }
    }

    let truncated = rows.len() > max_rows || truncated_by_bytes;
    if rows.len() > max_rows {
        rows.pop();
    }

    Ok((columns, rows, truncated))
}

fn is_interrupt_error(err: &sqlx::Error) -> bool {
    if let sqlx::Error::Database(db_err) = err {
        if db_err.code().as_deref() == Some("9") || db_err.code().as_deref() == Some("4") {
            return true;
        }
        let msg = db_err.message().to_lowercase();
        if msg.contains("interrupted") || msg.contains("interrupt") {
            return true;
        }
    }
    false
}
