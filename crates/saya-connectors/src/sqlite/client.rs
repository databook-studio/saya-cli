use std::{path::Path, time::Duration};

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
use tokio::time::timeout;

use crate::{ConnectorOptions, DatabaseConnector};

pub struct SqliteConnector {
    pub(crate) pool: SqlitePool,
    pub(crate) query_timeout: Duration,
    pub(crate) database: String,
}

impl SqliteConnector {
    pub async fn open(
        path: &Path,
        read_only: bool,
        settings: ConnectorOptions,
    ) -> Result<Self, ConnectionError> {
        if path.to_str() == Some(":memory:") {
            return Err(ConnectionError::InvalidConfiguration(
                "SQLite :memory: is not supported; use a file path".into(),
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
        })
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
        .map_err(|_| ConnectionError::ConnectionFailed("SQLite connection timed out".into()))?
        .map_err(super::errors::connection)?;
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        super::metadata::schema(self).await
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        super::execute::query(self, request).await
    }
}
