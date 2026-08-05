use async_trait::async_trait;
use saya_agent::{ToolDefinition, ToolExecutor};
use saya_store::SqliteStateStore;

#[cfg(test)]
use crate::connection::ConnectionEntry;
use crate::connection::ConnectionRegistry;
#[cfg(test)]
use saya_connectors::DatabaseConnector;

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    registry: ConnectionRegistry,
    max_rows: usize,
    allow_query_data: bool,
    state_db: Option<SqliteStateStore>,
}

impl DatabaseTools {
    /// Creates database tools with a single optional primary connection for testing.
    #[cfg(test)]
    pub(crate) fn new(
        connector: Option<Box<dyn DatabaseConnector>>,
        max_rows: usize,
        allow_query_data: bool,
    ) -> Self {
        let mut registry = ConnectionRegistry::new("primary");
        if let Some(c) = connector {
            let dialect = c.dialect();
            registry.insert(
                "primary",
                ConnectionEntry {
                    connector: c,
                    dialect,
                    profile_id: None,
                },
            );
        }
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db: None,
        }
    }

    /// Creates database tools configured with a connection registry.
    pub(crate) fn with_registry(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        state_db: Option<SqliteStateStore>,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db,
        }
    }

    /// Returns available database tool definitions.
    pub(crate) fn definitions(allow_query_data: bool) -> Vec<ToolDefinition> {
        let connection_prop = serde_json::json!({
            "type": "string",
            "description": "Optional. Name of the database connection to target; defaults to the primary. Available connections and their dialects are listed in the system context."
        });

        let mut tools = vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "Inspect the selected database schema without changing data.".into(),
            read_only: true,
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "connection": connection_prop
                },
                "additionalProperties": false
            }),
            requires_approval: false,
        }];
        if allow_query_data {
            tools.push(ToolDefinition {
                name: "bounded_sql_query".into(),
                description: "Run one bounded read-only SQL query against the selected database."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "connection": connection_prop,
                        "sql": {
                            "type": "string"
                        }
                    },
                    "required": ["sql"],
                    "additionalProperties": false
                }),
                requires_approval: true,
            });
        }
        tools
    }
}

#[async_trait]
impl ToolExecutor for DatabaseTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if name == "bounded_sql_query" && !self.allow_query_data {
            return Err("data sharing is disabled for this cloud provider".into());
        }
        let connection = arguments.get("connection").and_then(|v| v.as_str());
        let entry = self.registry.resolve(connection)?;
        match name {
            "schema_discovery" => {
                super::state_tools::schema(
                    entry.connector.as_ref(),
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await
            }
            "bounded_sql_query" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("invalid query arguments")?;
                super::state_tools::query(
                    entry.connector.as_ref(),
                    sql,
                    self.max_rows,
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await
            }
            _ => Err("unsupported read-only tool".into()),
        }
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;
