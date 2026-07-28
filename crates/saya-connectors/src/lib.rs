//! Database connector contracts for SAYA CLI.

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

mod duckdb;
mod factory;
mod mysql;
mod postgres;
mod safety;

pub use duckdb::DuckDbConnector;
pub use factory::{ConnectorOptions, build_connector};
pub use mysql::MySqlConnector;
pub use postgres::PostgresConnector;
pub use safety::{prepare_duckdb_sql, prepare_mysql_sql, prepare_postgres_sql};

/// Engine-neutral contract implemented by every SAYA database driver.
#[async_trait]
pub trait DatabaseConnector: Send + Sync {
    fn dialect(&self) -> SqlDialect;
    async fn connect(&self) -> Result<(), ConnectionError>;
    async fn schema(&self) -> Result<SchemaTree, ConnectionError>;
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError>;
    async fn cancel(&self) -> Result<(), ConnectionError> {
        Err(ConnectionError::Unsupported("query cancellation".into()))
    }
}
