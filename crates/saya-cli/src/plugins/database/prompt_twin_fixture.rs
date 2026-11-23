//! Shared fixture for the two live-prompt twins: a registry of fixed-dialect
//! connections and the section the live chat plugin states for it.

use saya_config::MemoryMode;
use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

use super::prompt::ChatPrompt;
use crate::connection::{ConnectionEntry, ConnectionRegistry};

/// A connector that answers nothing: the prompt only reads the dialect.
struct FixedDialect(SqlDialect);

#[async_trait::async_trait]
impl saya_connectors::DatabaseConnector for FixedDialect {
    fn dialect(&self) -> SqlDialect {
        self.0
    }
    async fn connect(&self) -> Result<(), ConnectionError> {
        Ok(())
    }
    async fn schema(&self) -> Result<SchemaTree, ConnectionError> {
        Ok(SchemaTree::default())
    }
    async fn execute(&self, request: QueryRequest) -> Result<QueryResult, ConnectionError> {
        Ok(QueryResult::empty(request.sql))
    }
}

/// A registry of `(name, dialect)` connections, in session order.
pub(super) fn registry(connections: &[(&str, SqlDialect)]) -> ConnectionRegistry {
    let primary = connections
        .first()
        .map(|(name, _)| *name)
        .unwrap_or("local");
    let mut registry = ConnectionRegistry::new(primary);
    for (name, dialect) in connections {
        registry.insert(
            *name,
            ConnectionEntry {
                connector: Box::new(FixedDialect(*dialect)),
                dialect: *dialect,
                profile_id: None,
            },
        );
    }
    registry
}

/// The live chat section for one connection set.
pub(super) fn section(
    connections: &[(&str, SqlDialect)],
    memory: MemoryMode,
    reachable: bool,
) -> String {
    ChatPrompt::of(&registry(connections), memory, reachable).section()
}
