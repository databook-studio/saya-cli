use std::{sync::Arc, time::Duration};

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use sqlx::{
    PgPool,
    postgres::{PgConnectOptions, PgPoolOptions},
};
use tokio::{sync::Mutex, time::timeout};

use crate::{ConnectorOptions, DatabaseConnector};

pub struct PostgresConnector {
    pub(crate) pool: PgPool,
    pub(crate) query_timeout: Duration,
    /// Serializes executes so the single active-query ID used for cancellation is unambiguous.
    pub(crate) in_flight: Arc<Mutex<()>>,
    pub(crate) active_pid: Arc<Mutex<Option<i32>>>,
}

impl PostgresConnector {
    pub fn from_options(mut options: PgConnectOptions, settings: ConnectorOptions) -> Self {
        if settings.read_only {
            options = options.options([("default_transaction_read_only", "on")]);
        }
        let query_timeout = Duration::from_secs(settings.query_timeout_seconds.max(1));
        let pool = PgPoolOptions::new()
            .max_connections(settings.max_connections.max(1))
            .acquire_timeout(query_timeout)
            .connect_lazy_with(options);
        Self {
            pool,
            query_timeout,
            in_flight: Arc::new(Mutex::new(())),
            active_pid: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl DatabaseConnector for PostgresConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Postgres
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        timeout(
            self.query_timeout,
            sqlx::query("SELECT 1").execute(&self.pool),
        )
        .await
        .map_err(|_| ConnectionError::ConnectionFailed("PostgreSQL connection timed out".into()))?
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

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::postgres::PgConnectOptions;

    #[tokio::test]
    async fn test_in_flight_mutex_serializes() {
        let connector =
            PostgresConnector::from_options(PgConnectOptions::new(), ConnectorOptions::default());

        let guard = connector.in_flight.try_lock();
        assert!(
            guard.is_ok(),
            "in_flight mutex should be initially unlocked"
        );

        let second_guard = connector.in_flight.try_lock();
        assert!(
            second_guard.is_err(),
            "in_flight mutex should be locked while guard held"
        );

        drop(guard);
        let third_guard = connector.in_flight.try_lock();
        assert!(
            third_guard.is_ok(),
            "in_flight mutex should be lockable after drop"
        );
    }
}
