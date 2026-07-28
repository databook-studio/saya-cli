use async_trait::async_trait;
use saya_agent::{ToolDefinition, ToolExecutor};
use saya_connectors::DatabaseConnector;
use saya_types::QueryRequest;

pub(crate) struct DatabaseTools {
    connector: Option<Box<dyn DatabaseConnector>>,
    max_rows: usize,
    allow_query_data: bool,
}

impl DatabaseTools {
    pub(crate) fn new(
        connector: Option<Box<dyn DatabaseConnector>>,
        max_rows: usize,
        allow_query_data: bool,
    ) -> Self {
        Self {
            connector,
            max_rows,
            allow_query_data,
        }
    }

    pub(crate) fn definitions(allow_query_data: bool) -> Vec<ToolDefinition> {
        let mut tools = vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "Inspect the selected PostgreSQL schema without changing data.".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
            requires_approval: false,
        }];
        if allow_query_data {
            tools.push(ToolDefinition {
                name: "bounded_sql_query".into(),
                description:
                    "Run one bounded read-only SQL query against the selected PostgreSQL profile."
                        .into(),
                read_only: true,
                parameters: serde_json::json!({"type":"object","properties":{"sql":{"type":"string"}},"required":["sql"],"additionalProperties":false}),
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
        let connector = self
            .connector
            .as_ref()
            .ok_or("no database profile is selected")?;
        match name {
            "schema_discovery" => serde_json::to_value(
                connector
                    .schema()
                    .await
                    .map_err(|_| "schema discovery failed")?,
            )
            .map_err(|_| "schema result unavailable".into()),
            "bounded_sql_query" => {
                let sql = arguments
                    .get("sql")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("invalid query arguments")?;
                let result = connector
                    .execute(QueryRequest::new(sql, self.max_rows))
                    .await
                    .map_err(|_| "read-only query failed")?;
                serde_json::to_value(result).map_err(|_| "query result unavailable".into())
            }
            _ => Err("unsupported read-only tool".into()),
        }
    }
}
