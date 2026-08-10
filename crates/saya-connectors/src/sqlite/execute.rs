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
        handle.set_progress_handler(1000, move || Instant::now() >= deadline);
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

    while let Some(row) = stream.try_next().await? {
        if columns.is_empty() {
            columns = row
                .columns()
                .iter()
                .map(|column| column.name().to_string())
                .collect();
        }
        let row_values = (0..row.len())
            .map(|index| decode::json_value(&row, index))
            .collect::<Result<Vec<_>, _>>()?;
        rows.push(Value::Array(row_values));

        if rows.len() > max_rows {
            break;
        }
    }

    let collected = rows.len();
    let truncated = collected > max_rows;
    if truncated {
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
