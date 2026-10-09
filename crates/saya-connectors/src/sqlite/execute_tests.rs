//! Tests for the SQLite execute path: cancellation, timeouts, the
//! parameter-free prepare path, and the native parameter binding.

#[path = "execute_tests/binding.rs"]
mod binding;
#[path = "execute_tests/cancellation.rs"]
mod cancellation;
#[path = "execute_tests/recovery.rs"]
mod recovery;

use std::sync::Arc;

use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};

use crate::{ConnectorOptions, SqliteConnector};

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
