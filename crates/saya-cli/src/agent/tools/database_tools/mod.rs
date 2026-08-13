use std::time::Duration;

use saya_agent::ToolError;
use saya_store::SqliteStateStore;

use crate::connection::ConnectionRegistry;

use definitions::validate_arguments;

mod chart_tool;
mod definitions;
mod fan_out;

/// Agent tools for inspecting and querying configured database connections.
pub(crate) struct DatabaseTools {
    // `pub(super)` so the sibling `contract_tools` module (also a child of
    // `agent::tools`) can resolve a connection and read the privacy/store flags
    // without re-deriving provider policy.
    pub(super) registry: ConnectionRegistry,
    pub(super) max_rows: usize,
    pub(super) allow_query_data: bool,
    pub(super) state_db: Option<SqliteStateStore>,
    pub(super) max_concurrent_fan_out_queries: usize,
    pub(super) fan_out_query_timeout: Duration,
}

impl DatabaseTools {
    const MAX_CONCURRENT_FAN_OUT_QUERIES: usize = 4;
    const FAN_OUT_QUERY_TIMEOUT: Duration = Duration::from_secs(30);

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
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
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
            max_concurrent_fan_out_queries: Self::MAX_CONCURRENT_FAN_OUT_QUERIES,
            fan_out_query_timeout: Self::FAN_OUT_QUERY_TIMEOUT,
        }
    }

    #[cfg(test)]
    pub(super) fn with_registry_and_fan_out_limits(
        registry: ConnectionRegistry,
        max_rows: usize,
        allow_query_data: bool,
        max_concurrent_fan_out_queries: usize,
        fan_out_query_timeout: Duration,
    ) -> Self {
        Self {
            registry,
            max_rows,
            allow_query_data,
            state_db: None,
            max_concurrent_fan_out_queries: max_concurrent_fan_out_queries.max(1),
            fan_out_query_timeout,
        }
    }

    /// Dispatches a read-only agent tool call to its selected connection.
    pub(super) async fn execute_read_only(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        // Contract tools have their own argument validation and execution
        // (sibling concern) and never reach a connector; route them before the
        // database-tool validation, which would reject their names.
        if matches!(name, "contract_search" | "contract_read") {
            return self.execute_contract_tool(name, arguments).await;
        }
        validate_arguments(name, &arguments)?;
        if matches!(
            name,
            "bounded_sql_query" | "bounded_sql_query_all" | "render_chart"
        ) && !self.allow_query_data
        {
            return Err(ToolError::DataSharingDisabled);
        }
        if name == "bounded_sql_query_all" {
            let sql = arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .ok_or(ToolError::InvalidQueryArguments)?;
            return self.query_all(sql).await;
        }
        let connection = arguments
            .get("connection")
            .and_then(serde_json::Value::as_str);
        let entry = self.registry.resolve(connection)?;
        match name {
            "schema_discovery" => {
                crate::agent::state_tools::schema(
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
                    .ok_or(ToolError::InvalidQueryArguments)?;
                crate::agent::state_tools::query(
                    entry.connector.as_ref(),
                    sql,
                    self.max_rows,
                    self.state_db.as_ref(),
                    entry.profile_id.as_deref(),
                )
                .await
            }
            "render_chart" => self.render_chart(&arguments).await,
            _ => Err(ToolError::UnsupportedTool),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_agent::ToolExecutor;

    #[test]
    fn render_chart_requires_approval() {
        let tools = DatabaseTools::definitions(true, false);
        let chart_tool = tools
            .iter()
            .find(|tool| tool.name == "render_chart")
            .expect("render_chart definition exists");
        assert!(chart_tool.effect.requires_approval);
        assert!(chart_tool.effect.external_side_effect);
        assert!(!chart_tool.effect.database_data);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn render_chart_creates_0600_permissions_file() {
        use async_trait::async_trait;
        use saya_connectors::DatabaseConnector;
        use saya_types::{ConnectionError, QueryRequest, QueryResult, SchemaTree, SqlDialect};

        struct NonEmptyConnector;

        #[async_trait]
        impl DatabaseConnector for NonEmptyConnector {
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
                Ok(QueryResult {
                    columns: vec!["cat".into(), "val".into()],
                    rows: vec![serde_json::json!(["A", 10])],
                    row_count: 1,
                    truncated: false,
                    executed_sql: req.sql,
                })
            }
        }

        let tools = DatabaseTools::new(Some(Box::new(NonEmptyConnector)), 100, true);
        let res = tools
            .execute(
                "render_chart",
                serde_json::json!({"sql": "SELECT 1", "chart_type": "bar"}),
            )
            .await
            .expect("render_chart should succeed");

        let path_str = res["path"].as_str().expect("path in response");
        let path = std::path::Path::new(path_str);
        let meta = std::fs::metadata(path).expect("file should exist");

        use std::os::unix::fs::PermissionsExt;
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let _ = std::fs::remove_file(path);
    }
}
