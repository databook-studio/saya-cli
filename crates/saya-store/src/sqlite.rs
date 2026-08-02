use crate::{StoreError, sqlite_support};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::OnceCell;

#[derive(Clone)]
pub struct SqliteStateStore {
    path: Arc<PathBuf>,
    pool: Arc<OnceCell<SqlitePool>>,
}

impl SqliteStateStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: Arc::new(path.into()),
            pool: Arc::new(OnceCell::new()),
        }
    }

    pub(crate) async fn pool(&self) -> Result<&SqlitePool, StoreError> {
        self.pool
            .get_or_try_init(|| async {
                sqlite_support::prepare_path(&self.path)?;
                let options = SqliteConnectOptions::new()
                    .filename(&*self.path)
                    .create_if_missing(true)
                    .journal_mode(SqliteJournalMode::Wal)
                    .busy_timeout(Duration::from_secs(5))
                    .foreign_keys(true);
                let pool = SqlitePoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(Duration::from_secs(5))
                    .connect_with(options)
                    .await
                    .map_err(|_| StoreError::Unavailable)?;
                migrate(&pool).await?;
                sqlite_support::secure_files(&self.path)?;
                Ok(pool)
            })
            .await
    }
    pub(crate) fn secure_files(&self) -> Result<(), StoreError> {
        sqlite_support::secure_files(&self.path)
    }
}

async fn migrate(pool: &SqlitePool) -> Result<(), StoreError> {
    let mut tx = pool.begin().await.map_err(|_| StoreError::Unavailable)?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    match version {
        0 => {
            sqlx::query("CREATE TABLE schema_cache(profile_id TEXT PRIMARY KEY, schema_json TEXT NOT NULL, updated_unix_ms INTEGER NOT NULL, version INTEGER NOT NULL)").execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
            sqlx::query("CREATE TABLE audit_log(id INTEGER PRIMARY KEY, created_unix_ms INTEGER NOT NULL, session_id TEXT, profile_id TEXT NOT NULL, operation TEXT NOT NULL, status TEXT NOT NULL, duration_ms INTEGER NOT NULL, row_count INTEGER, truncated INTEGER)").execute(&mut *tx).await.map_err(|_| StoreError::Unavailable)?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&mut *tx)
                .await
                .map_err(|_| StoreError::Unavailable)?;
        }
        1 => {}
        _ => return Err(StoreError::Unavailable),
    }
    tx.commit().await.map_err(|_| StoreError::Unavailable)
}
