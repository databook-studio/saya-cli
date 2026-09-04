//! Database connector contracts for SAYA CLI.

use async_trait::async_trait;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

mod clickhouse;
mod common;
mod duckdb;
mod factory;
mod mysql;
mod postgres;
mod safety;
mod snowflake;
mod sqlite;

pub use clickhouse::ClickHouseConnector;
pub use duckdb::DuckDbConnector;
pub use factory::{ConnectorOptions, build_connector, build_connector_with_prompt};
pub use mysql::MySqlConnector;
pub use postgres::PostgresConnector;
pub use safety::{
    SqlReferences, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_mysql_sql,
    prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql, sql_references,
};
pub use snowflake::SnowflakeConnector;
pub use sqlite::SqliteConnector;

/// Engine-neutral contract implemented by every SAYA database driver.
#[async_trait]
pub trait DatabaseConnector: Send + Sync {
    fn dialect(&self) -> SqlDialect;
    async fn connect(&self) -> Result<(), ConnectionError>;
    async fn schema(&self) -> Result<SchemaTree, ConnectionError>;
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError>;
    async fn cancel(&self) -> Result<(), ConnectionError> {
        Err(ConnectionError::unsupported("query cancellation"))
    }
}
