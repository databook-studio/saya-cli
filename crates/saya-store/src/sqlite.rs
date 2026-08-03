use crate::{StoreError, sqlite_support};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::OnceCell;

const LOCK_RETRIES: usize = 100;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(50);

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
    let mut connection = pool.acquire().await.map_err(|_| StoreError::Unavailable)?;
    retry_statement(&mut connection, "BEGIN IMMEDIATE").await?;
    let version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&mut *connection)
        .await
        .map_err(|_| StoreError::Unavailable)?;
    let created = match version {
        0 => {
            sqlx::query("CREATE TABLE IF NOT EXISTS schema_cache(profile_id TEXT PRIMARY KEY, schema_json TEXT NOT NULL, updated_unix_ms INTEGER NOT NULL, version INTEGER NOT NULL)").execute(&mut *connection).await.map_err(|_| StoreError::Unavailable)?;
            sqlx::query("CREATE TABLE IF NOT EXISTS audit_log(id INTEGER PRIMARY KEY, created_unix_ms INTEGER NOT NULL, session_id TEXT, profile_id TEXT NOT NULL, operation TEXT NOT NULL, status TEXT NOT NULL, duration_ms INTEGER NOT NULL, row_count INTEGER, truncated INTEGER)").execute(&mut *connection).await.map_err(|_| StoreError::Unavailable)?;
            sqlx::query("PRAGMA user_version = 1")
                .execute(&mut *connection)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            true
        }
        1 => false,
        _ => {
            sqlx::query("ROLLBACK").execute(&mut *connection).await.ok();
            return Err(StoreError::Unavailable);
        }
    };
    sqlx::query("COMMIT")
        .execute(&mut *connection)
        .await
        .map(|_| ())
        .map_err(|_| StoreError::Unavailable)?;
    if created {
        retry_statement(&mut connection, "PRAGMA journal_mode = WAL").await?;
    }
    Ok(())
}

async fn retry_statement(
    connection: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    statement: &str,
) -> Result<(), StoreError> {
    for _ in 0..LOCK_RETRIES {
        if sqlx::query(statement)
            .execute(&mut **connection)
            .await
            .is_ok()
        {
            return Ok(());
        }
        tokio::time::sleep(LOCK_RETRY_DELAY).await;
    }
    Err(StoreError::Unavailable)
}
