use std::time::Instant;

use futures_util::TryStreamExt;
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;
use sqlx::{Column as _, Row};
use std::sync::atomic::Ordering;
use tokio::time::timeout;

use super::{SqliteConnector, decode, errors};

pub(crate) async fn query(
    c: &SqliteConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_sqlite_sql(&request.sql, request.max_rows)?;

    let mut conn = timeout(c.query_timeout, c.pool.acquire())
        .await
        .map_err(|_| ConnectionError::query_failed("SQLite query timed out"))?
        .map_err(errors::query)?;

    let deadline = Instant::now() + c.query_timeout;
    let cancelled = c.cancelled.clone();
    // Cleared before each run so a connector that was cancelled once stays usable
    // for the next query rather than aborting it immediately.
    cancelled.store(false, Ordering::Release);

    {
        let mut handle = conn.lock_handle().await.map_err(errors::query)?;
        // The handler keeps the query going only while inside the deadline and not
        // cancelled. Returning `false` aborts the running statement from inside
        // the SQLite VM — the same mechanism the deadline already used.
        handle.set_progress_handler(1000, move || {
            Instant::now() < deadline && !cancelled.load(Ordering::Acquire)
        });
    }

    let stream_res = fetch_rows(&mut conn, &sql, request.max_rows).await;

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

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use saya_types::{ConnectionError, QueryRequest};
    use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

    use crate::{ConnectorOptions, DatabaseConnector, SqliteConnector};

    /// A query that keeps the SQLite VM busy long enough to be interrupted. The
    /// recursive CTE produces far more rows than the timeout or a cancellation
    /// will let it finish.
    const SLOW_QUERY: &str = "WITH RECURSIVE cnt(x) AS (SELECT 1 UNION ALL SELECT x+1 FROM cnt WHERE x < 500000000) \
         SELECT count(*) FROM cnt";

    /// Opens a read-only connector against a fresh temp file. The temp dir is
    /// returned so the caller keeps the file path alive for the pool, which
    /// opens further connections by path on demand. The file is seeded first:
    /// SQLite refuses a read-only open of a path that does not exist.
    async fn open(timeout_seconds: u64) -> (Arc<SqliteConnector>, tempfile::TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("cancel.db");
        let seed = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        seed.close().await;
        let connector = Arc::new(
            SqliteConnector::open(
                &db,
                true,
                ConnectorOptions {
                    query_timeout_seconds: timeout_seconds,
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
        );
        (connector, dir)
    }

    #[tokio::test]
    async fn cancelled_query_reports_cancellation_not_timeout() {
        let (connector, _dir) = open(30).await;
        let runner = connector.clone();
        let task = tokio::spawn(async move {
            runner
                .execute(QueryRequest::new(SLOW_QUERY.to_string(), 1))
                .await
        });
        // Let the statement start before cancelling it.
        tokio::time::sleep(Duration::from_millis(200)).await;
        connector.cancel().await.unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .expect("cancelled query stops within seconds")
            .unwrap();
        assert!(
            matches!(result, Err(ConnectionError::Cancelled)),
            "a cancelled query reports cancellation, not a timeout: {result:?}"
        );
    }

    #[tokio::test]
    async fn deadline_path_still_reports_timeout() {
        let (connector, _dir) = open(1).await;
        let result = connector
            .execute(QueryRequest::new(SLOW_QUERY.to_string(), 1))
            .await;
        let err = result.expect_err("the deadline fires before the query finishes");
        let message = err.to_string().to_lowercase();
        assert!(
            message.contains("timed out"),
            "the deadline path reports a timeout: {message}"
        );
        assert!(
            !message.contains("cancel"),
            "a timed-out query is not reported as cancelled: {message}"
        );
    }

    #[tokio::test]
    async fn connection_works_for_the_next_query_after_cancellation() {
        let dir = tempfile::TempDir::new().unwrap();
        let db = dir.path().join("reuse.db");
        let seed = SqlitePool::connect_with(
            SqliteConnectOptions::new()
                .filename(&db)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::query("CREATE TABLE t (id INTEGER)")
            .execute(&seed)
            .await
            .unwrap();
        sqlx::query("INSERT INTO t VALUES (1), (2), (3)")
            .execute(&seed)
            .await
            .unwrap();
        seed.close().await;

        let connector = Arc::new(
            SqliteConnector::open(
                &db,
                true,
                ConnectorOptions {
                    query_timeout_seconds: 30,
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
        );

        let runner = connector.clone();
        let task = tokio::spawn(async move {
            runner
                .execute(QueryRequest::new(SLOW_QUERY.to_string(), 1))
                .await
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        connector.cancel().await.unwrap();
        let cancelled = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(cancelled, Err(ConnectionError::Cancelled)));

        // The same connector must serve the next query — no poisoned pool entry.
        let result = connector
            .execute(QueryRequest::new(
                "SELECT id FROM t ORDER BY id".to_string(),
                10,
            ))
            .await
            .unwrap();
        assert_eq!(result.row_count, 3);
    }
}
