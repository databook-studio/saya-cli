use super::{compact_schema, query_with_context, schema};
use async_trait::async_trait;
use saya_connectors::DatabaseConnector;
use saya_store::{AuditOperation, AuditStore, SchemaStore, SqliteStateStore};
use saya_types::{
    Column, ConnectionError, Database, ForeignKey, QueryRequest, QueryResult, Schema, SchemaTree,
    SqlDialect, Table,
};
use std::{
    fs,
    time::{SystemTime, UNIX_EPOCH},
};

use crate::connection::{ConnectionEntry, ConnectionRegistry};

struct Failing;
#[async_trait]
impl DatabaseConnector for Failing {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::schema_failed("server sentinel"))
    }
    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("row sentinel"))
    }
}

include!("state_tools_tests/query_audit.rs");

include!("state_tools_tests/schema_keys.rs");

include!("state_tools_tests/schema_collisions.rs");
