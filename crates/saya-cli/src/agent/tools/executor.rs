use async_trait::async_trait;
use saya_agent::ToolExecutor;

use super::database_tools::DatabaseTools;

#[async_trait]
impl ToolExecutor for DatabaseTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        if matches!(name, "bounded_sql_query" | "bounded_sql_query_all") && !self.allow_query_data {
            return Err("data sharing is disabled for this cloud provider".into());
        }
        if name == "bounded_sql_query_all" {
            let sql = arguments
                .get("sql")
                .and_then(serde_json::Value::as_str)
                .ok_or("invalid query arguments")?;
            return self.query_all(sql).await;
        }
        let connection = arguments.get("connection").and_then(|v| v.as_str());
        let entry = self.registry.resolve(connection)?;
        match name {
            "schema_discovery" => {
                super::super::state_tools::schema(
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
                super::super::state_tools::query(
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
