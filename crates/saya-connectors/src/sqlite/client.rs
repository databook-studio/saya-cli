use std::{
    path::Path,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::Duration,
};

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use tokio::time::timeout;

use crate::{CancelRequestOutcome, ConnectorOptions, DatabaseConnector};

pub struct SqliteConnector {
    pub(crate) pool: SqlitePool,
    pub(crate) query_timeout: Duration,
    pub(crate) database: String,
    /// Shared with the progress-handler closure installed by `execute`. `cancel`
    /// sets it so the handler returns `false` (the same signal a missed deadline
    /// sends), aborting the running statement from inside the SQLite VM.
    pub(crate) cancelled: Arc<AtomicBool>,
    /// Counts attempts from before pool acquisition through execute settlement.
    /// It keeps overlapping executes from clearing one another's request.
    pub(crate) active_attempts: Mutex<usize>,
}

pub(crate) struct QueryAttempt<'a>(&'a SqliteConnector);

impl Drop for QueryAttempt<'_> {
    fn drop(&mut self) {
        let mut active = self
            .0
            .active_attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *active -= 1;
    }
}

impl SqliteConnector {
    pub async fn open(
        path: &Path,
        read_only: bool,
        settings: ConnectorOptions,
    ) -> Result<Self, ConnectionError> {
        if path.to_str() == Some(":memory:") {
            return Err(ConnectionError::invalid_configuration(
                "SQLite :memory: is not supported; use a file path",
            ));
        }

        let query_timeout = Duration::from_secs(settings.query_timeout_seconds.max(1));
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .read_only(read_only)
            .create_if_missing(false)
            .pragma("query_only", "ON")
            .pragma("trusted_schema", "OFF")
            .busy_timeout(query_timeout);

        let pool = SqlitePoolOptions::new()
            .max_connections(settings.max_connections.max(1))
            .acquire_timeout(query_timeout)
            .connect_with(options)
            .await
            .map_err(super::errors::connection)?;

        let database = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main".to_string());

        Ok(Self {
            pool,
            query_timeout,
            database,
            cancelled: Arc::new(AtomicBool::new(false)),
            active_attempts: Mutex::new(0),
        })
    }

    pub(crate) fn begin_attempt(&self) -> QueryAttempt<'_> {
        let mut active = self
            .active_attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *active == 0 {
            self.cancelled
                .store(false, std::sync::atomic::Ordering::Release);
        }
        *active += 1;
        QueryAttempt(self)
    }

    fn request_cancel(&self) -> CancelRequestOutcome {
        let active = self
            .active_attempts
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *active == 0 {
            return CancelRequestOutcome::NoActiveOperation;
        }
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Release);
        CancelRequestOutcome::LocalInterruptRequested
    }
}

#[async_trait]
impl DatabaseConnector for SqliteConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        timeout(
            self.query_timeout,
            sqlx::query("SELECT 1").execute(&self.pool),
        )
        .await
        .map_err(|_| ConnectionError::connection_failed("SQLite connection timed out"))?
        .map_err(super::errors::connection)?;
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        super::metadata::schema(self).await
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        super::execute::query(self, request).await
    }

    fn supports_parameters(&self) -> bool {
        true
    }

    async fn cancel(&self) -> Result<(), ConnectionError> {
        self.request_cancel();
        Ok(())
    }

    async fn request_cancel(&self) -> Result<CancelRequestOutcome, ConnectionError> {
        Ok(self.request_cancel())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;

    #[tokio::test]
    async fn test_production_pool_query_only_and_trusted_schema() {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let db_path = temp_dir.path().join("prod_pool_test.db");

        let fixture_options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let fixture_pool = SqlitePool::connect_with(fixture_options).await.unwrap();
        sqlx::query("CREATE TABLE t (id INT);")
            .execute(&fixture_pool)
            .await
            .unwrap();
        fixture_pool.close().await;

        let opts = ConnectorOptions::default();
        let connector = SqliteConnector::open(&db_path, false, opts).await.unwrap();

        let insert_res = sqlx::query("INSERT INTO t VALUES (1)")
            .execute(&connector.pool)
            .await;
        assert!(
            insert_res.is_err(),
            "query_only=ON pragma must block writes even when file opened with read_only=false"
        );

        let row: (i64,) = sqlx::query_as("PRAGMA trusted_schema")
            .fetch_one(&connector.pool)
            .await
            .unwrap();
        assert_eq!(row.0, 0, "PRAGMA trusted_schema must be 0");

        connector.pool.close().await;
        drop(connector);
        drop(temp_dir);
    }
}
