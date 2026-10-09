use super::*;
use async_trait::async_trait;
use saya_agent::ToolExecutor;
use saya_connectors::DatabaseConnector;
use saya_types::{
    ConnectionError, Database, QueryRequest, QueryResult, Schema, SchemaTree, SqlDialect, Table,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use std::{future::poll_fn, task::Poll};
use tokio::sync::{Barrier, Notify};

struct FakeConnector {
    table_name: String,
}

#[async_trait]
impl DatabaseConnector for FakeConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree {
            databases: vec![Database {
                name: "main".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![Table {
                        name: self.table_name.clone(),
                        primary_key: vec![],
                        foreign_keys: vec![],
                        columns: vec![],
                    }],
                }],
            }],
        })
    }

    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(req.sql))
    }
}

/// A connector whose queries always fail, used to exercise the fan-out
/// per-database error path (e.g. a dialect mismatch on one database).
struct FailingConnector {
    dialect: SqlDialect,
}

struct BarrierConnector {
    barrier: Arc<Barrier>,
}

struct SlowConnector;

struct LeaseConnector {
    started: Arc<Notify>,
    release: Arc<Notify>,
    executions: Arc<AtomicUsize>,
    cancel_requests: Arc<AtomicUsize>,
}

#[async_trait]
impl DatabaseConnector for SlowConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok(QueryResult::empty("SELECT 1"))
    }
}

#[async_trait]
impl DatabaseConnector for LeaseConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::Sqlite
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        self.started.notify_waiters();
        self.release.notified().await;
        Ok(QueryResult::empty(request.sql))
    }

    async fn request_cancel(
        &self,
    ) -> Result<saya_connectors::CancelRequestOutcome, ConnectionError> {
        self.cancel_requests.fetch_add(1, Ordering::SeqCst);
        Ok(saya_connectors::CancelRequestOutcome::LocalInterruptRequested)
    }
}

#[async_trait]
impl DatabaseConnector for BarrierConnector {
    fn dialect(&self) -> SqlDialect {
        SqlDialect::DuckDb
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }

    async fn execute(&self, req: QueryRequest) -> Result<QueryResult, ConnectionError> {
        self.barrier.wait().await;
        Ok(QueryResult::empty(req.sql))
    }
}

#[async_trait]
impl DatabaseConnector for FailingConnector {
    fn dialect(&self) -> SqlDialect {
        self.dialect
    }

    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }

    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Err(ConnectionError::schema_failed("nope"))
    }

    async fn execute(&self, _: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Err(ConnectionError::query_failed("syntax error near FROM"))
    }
}

fn two_connection_registry() -> ConnectionRegistry {
    let mut registry = ConnectionRegistry::new("primary");
    registry.insert(
        "primary",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_primary".into(),
            }),
            dialect: SqlDialect::DuckDb,
            profile_id: None,
        },
    );
    registry.insert(
        "warehouse",
        ConnectionEntry {
            connector: Box::new(FakeConnector {
                table_name: "from_secondary".into(),
            }),
            dialect: SqlDialect::Postgres,
            profile_id: None,
        },
    );
    registry
}

include!("tools_tests/fanout_results.rs");

include!("tools_tests/fanout_limits.rs");

include!("tools_tests/registry_cancellation.rs");

include!("tools_tests/sharing_and_schemas.rs");

include!("tools_tests/render_and_answer.rs");

include!("tools_tests/tool_effects.rs");

include!("tools_tests/detail_and_routing.rs");
