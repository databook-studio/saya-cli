use saya_agent::ToolDefinition;
use saya_store::SqliteStateStore;

use crate::connection::ConnectionRegistry;

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    pub(super) registry: ConnectionRegistry,
    pub(super) max_rows: usize,
    pub(super) allow_query_data: bool,
    pub(super) state_db: Option<SqliteStateStore>,
}

impl DatabaseTools {
    /// Creates database tools with a single optional primary connection for testing.
    #[cfg(test)]
    pub(crate) fn new(
        connector: Option<Box<dyn saya_connectors::DatabaseConnector>>,
        max_rows: usize,
        allow_query_data: bool,
    ) -> Self {
        use crate::connection::ConnectionEntry;

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

    /// Runs `sql` against every connected database independently, collecting a
    /// per-database `result` or `error` so a dialect mismatch on one database
    /// never sinks the rest. A single approval covers the whole fan-out.
    pub(super) async fn query_all(&self, sql: &str) -> Result<serde_json::Value, String> {
        let entries = self.registry.entries();
        if entries.is_empty() {
            return Err("no database profile is selected".into());
        }
        let mut databases = Vec::with_capacity(entries.len());
        for (name, entry) in entries {
            let outcome = super::super::state_tools::query(
                entry.connector.as_ref(),
                sql,
                self.max_rows,
                self.state_db.as_ref(),
                entry.profile_id.as_deref(),
            )
            .await;
            let mut record = serde_json::Map::new();
            record.insert(
                "connection".into(),
                serde_json::Value::String(name.to_string()),
            );
            record.insert(
                "dialect".into(),
                serde_json::Value::String(entry.dialect.as_str().to_string()),
            );
            match outcome {
                Ok(result) => {
                    record.insert("result".into(), result);
                }
                Err(error) => {
                    record.insert("error".into(), serde_json::Value::String(error));
                }
            }
            databases.push(serde_json::Value::Object(record));
        }
        Ok(serde_json::json!({ "databases": databases }))
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
            tools.push(ToolDefinition {
                name: "bounded_sql_query_all".into(),
                description: "Run one bounded read-only SQL query against EVERY connected \
                    database at once and return the per-database results. Use this when the \
                    same question should be answered across all connected databases; each \
                    database runs independently, so a failure on one (e.g. a dialect \
                    mismatch) is reported alongside the successes rather than aborting the \
                    rest. Do not pass a `connection` argument."
                    .into(),
                read_only: true,
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
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
