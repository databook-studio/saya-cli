use crate::{StoreError, migration, sqlite_support};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
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
                    .busy_timeout(Duration::from_secs(5))
                    .foreign_keys(true);
                let pool = SqlitePoolOptions::new()
                    .max_connections(4)
                    .acquire_timeout(Duration::from_secs(5))
                    .connect_with(options)
                    .await
                    .map_err(|_| StoreError::Unavailable)?;
                migration::migrate(&pool).await?;
                sqlite_support::secure_files(&self.path)?;
                Ok(pool)
            })
            .await
    }
    pub async fn close(&self) {
        if let Some(pool) = self.pool.get() {
            pool.close().await;
        }
    }
    pub(crate) fn secure_files(&self) -> Result<(), StoreError> {
        sqlite_support::secure_files(&self.path)
    }
}
