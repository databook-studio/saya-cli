//! Database connector contracts for SAYA CLI.

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

mod bigquery;
mod binds;
mod clickhouse;
mod common;
mod duckdb;
mod factory;
mod mysql;
mod postgres;
mod safety;
mod snowflake;
mod sqlite;
mod verify;

pub use bigquery::BigQueryConnector;
pub use clickhouse::ClickHouseConnector;
pub use duckdb::DuckDbConnector;
pub use factory::{ConnectorOptions, build_connector, build_connector_with_prompt};
pub use mysql::MySqlConnector;
pub use postgres::PostgresConnector;
pub use safety::{
    PreparedQuery, SqlReferences, prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql,
    prepare_for_dialect, prepare_mysql_sql, prepare_postgres_sql, prepare_snowflake_sql,
    prepare_sqlite_sql, prepare_with_params, sql_placeholders, sql_references,
};
pub use snowflake::SnowflakeConnector;
pub use sqlite::SqliteConnector;
pub use verify::{FanoutProbe, fanout_probe, has_top_level_order_by};

/// What a connector knows after issuing a cancellation request.
///
/// This describes the request mechanism only; it does not mean the query has
/// reached a terminal state. Observe `execute` settlement for that evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CancelRequestOutcome {
    /// A local interrupt mechanism was signalled.
    LocalInterruptRequested,
    /// A remote service accepted the cancellation request.
    RemoteRequestAccepted,
    /// The connector has no operation to target at the time of the request.
    NoActiveOperation,
    /// A legacy `cancel` implementation succeeded, but its mechanism is unknown.
    LegacyOutcomeUnknown,
}

/// Engine-neutral contract implemented by every SAYA database driver.
#[async_trait]
pub trait DatabaseConnector: Send + Sync {
    fn dialect(&self) -> SqlDialect;
    async fn connect(&self) -> Result<(), ConnectionError>;
    async fn schema(&self) -> Result<SchemaTree, ConnectionError>;
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError>;
    /// Whether this connector binds [`QueryRequest::params`] natively. An
    /// engine without native binding refuses a non-empty parameter list with
    /// an unsupported error before any connection attempt; parameter-free
    /// queries work everywhere.
    fn supports_parameters(&self) -> bool {
        false
    }
    async fn cancel(&self) -> Result<(), ConnectionError> {
        Err(ConnectionError::unsupported("query cancellation"))
    }

    /// Requests cancellation and reports only what the connector mechanism
    /// establishes. Existing implementations remain compatible through this
    /// default, which preserves their `cancel` behavior without overstating it.
    async fn request_cancel(&self) -> Result<CancelRequestOutcome, ConnectionError> {
        self.cancel().await?;
        Ok(CancelRequestOutcome::LegacyOutcomeUnknown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct LegacyConnector;

    #[async_trait]
    impl DatabaseConnector for LegacyConnector {
        fn dialect(&self) -> SqlDialect {
            SqlDialect::Sqlite
        }

        async fn connect(&self) -> Result<(), ConnectionError> {
            Err(ConnectionError::unsupported("test connection"))
        }

        async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
            Err(ConnectionError::unsupported("test schema"))
        }

        async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
            Err(ConnectionError::unsupported("test query"))
        }

        async fn cancel(&self) -> Result<(), ConnectionError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn default_request_cancel_preserves_legacy_cancel_without_overclaiming() {
        assert_eq!(
            LegacyConnector.request_cancel().await.unwrap(),
            CancelRequestOutcome::LegacyOutcomeUnknown
        );
    }
}
