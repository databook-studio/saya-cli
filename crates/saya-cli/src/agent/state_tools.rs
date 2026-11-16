use saya_agent::{AgentMetricOutcome, ToolError, ToolExecutionContext};
use saya_connectors::DatabaseConnector;
use saya_store::{
    AuditEntry, AuditOperation, AuditStatus, AuditStore, SchemaStore, SqliteStateStore,
};
use saya_types::{ForeignKey, QueryRequest, QueryResult, SchemaTree, SqlDialect};
use std::collections::HashMap;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::connection::OperationEntry;

mod operation;
mod operation_metrics;

include!("state_tools/compact_schema.rs");

include!("state_tools/schema_discovery.rs");

include!("state_tools/query_execution.rs");

#[cfg(test)]
#[path = "state_tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "state_tools/operation_metrics_tests.rs"]
mod operation_metrics_tests;
