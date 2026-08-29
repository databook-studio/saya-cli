use std::time::Duration;

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use sqlx::{
    MySqlPool,
    mysql::{MySqlConnectOptions, MySqlPoolOptions},
};
use tokio::{sync::Mutex, time::timeout};

use crate::{ConnectorOptions, DatabaseConnector};

pub struct MySqlConnector {
    pub(crate) pool: MySqlPool,
    pub(crate) database: String,
    pub(crate) query_timeout: Duration,
    /// Connect options reused to open the *dedicated* `KILL QUERY` connection
    /// in `cancellation::cancel`. Kept here (rather than re-deriving from the
    /// pool) so the kill never competes with the timed-out query for a pooled
    /// connection — see `cancellation.rs`. Postgres issues its cancel over the
    /// shared pool; MySQL cannot, because a timed-out query holds its pooled
    /// connection (see `execute.rs`) and `max_connections` may be 1.
    pub(crate) kill_options: MySqlConnectOptions,
    /// Serializes executes so the single connection ID used for cancellation
    /// is unambiguous. `execute.rs` acquires one pooled connection, captures
    /// `CONNECTION_ID()` on it, and runs the query on the same connection —
    /// mirroring the Postgres connector's `pg_backend_pid()` shape.
    pub(crate) in_flight: Mutex<()>,
    pub(crate) active_id: Mutex<Option<u64>>,
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
        // Clone before the pool consumes the options: `cancel` opens its own
        // short-lived connection from these same options (same user, TLS, CA).
        let kill_options = options.clone();
        let pool = pool_options.connect_lazy_with(options);
        Self {
            pool,
            database: database.into(),
            query_timeout,
            kill_options,
            in_flight: Mutex::new(()),
            active_id: Mutex::new(None),
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
        .map_err(|_| ConnectionError::connection_failed("MySQL connection timed out"))?
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
        super::cancellation::cancel(self).await
    }
}
