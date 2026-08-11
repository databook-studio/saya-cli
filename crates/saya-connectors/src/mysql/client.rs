use std::time::Duration;

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use sqlx::{
    MySqlPool,
    mysql::{MySqlConnectOptions, MySqlPoolOptions},
};
use tokio::time::timeout;

use crate::{ConnectorOptions, DatabaseConnector};

pub struct MySqlConnector {
    pub(crate) pool: MySqlPool,
    pub(crate) database: String,
    pub(crate) query_timeout: Duration,
}

impl MySqlConnector {
    pub fn from_options(
        options: MySqlConnectOptions,
        database: &str,
        settings: ConnectorOptions,
    ) -> Self {
        let query_timeout = Duration::from_secs(settings.query_timeout_seconds.max(1));
        let mut pool_options = MySqlPoolOptions::new()
            .max_connections(settings.max_connections.max(1))
            .acquire_timeout(query_timeout);
        if settings.read_only {
            pool_options = pool_options.after_connect(|conn, _meta| {
                Box::pin(async move {
                    sqlx::query("SET SESSION transaction_read_only = 1")
                        .execute(conn)
                        .await?;
                    Ok(())
                })
            });
        }
        let pool = pool_options.connect_lazy_with(options);
        Self {
            pool,
            database: database.into(),
            query_timeout,
        }
    }
}

#[async_trait]
impl DatabaseConnector for MySqlConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Mysql
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        timeout(
            self.query_timeout,
            sqlx::query("SELECT 1").execute(&self.pool),
        )
        .await
        .map_err(|_| ConnectionError::ConnectionFailed("MySQL connection timed out".into()))?
        .map_err(super::errors::connection)?;
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        super::metadata::schema(self).await
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        super::execute::query(self, request).await
    }

    async fn cancel(&self) -> Result<(), ConnectionError> {
        Err(ConnectionError::Unsupported(
            "MySQL cancellation is not safely available".into(),
        ))
    }
}
