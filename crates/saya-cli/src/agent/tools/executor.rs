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
        self.execute_read_only(name, arguments).await
    }
}
