use std::time::Instant;

use futures_util::TryStreamExt;
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;
use sqlx::{Column as _, Row};
use tokio::time::timeout;

use super::{SqliteConnector, decode, errors};

pub(crate) async fn query(
    c: &SqliteConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_sqlite_sql(&request.sql, request.max_rows)?;

    let mut conn = timeout(c.query_timeout, c.pool.acquire())
        .await
        .map_err(|_| ConnectionError::QueryFailed("SQLite query timed out".into()))?
        .map_err(errors::query)?;

    let deadline = Instant::now() + c.query_timeout;

    {
        let mut handle = conn.lock_handle().await.map_err(errors::query)?;
        handle.set_progress_handler(1000, move || Instant::now() < deadline);
    }

    let stream_res = fetch_rows(&mut conn, &sql, request.max_rows).await;

    if let Ok(mut handle) = conn.lock_handle().await {
        handle.remove_progress_handler();
    }

    let (columns, rows, truncated) = match stream_res {
        Ok(res) => res,
        Err(err) => {
            if Instant::now() >= deadline || is_interrupt_error(&err) {
                return Err(ConnectionError::QueryFailed(
                    "SQLite query timed out".into(),
                ));
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

async fn fetch_rows(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    sql: &str,
    max_rows: usize,
) -> Result<(Vec<String>, Vec<Value>, bool), sqlx::Error> {
    let mut stream = sqlx::query(sql).fetch(&mut **conn);
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
