use async_trait::async_trait;
use saya_agent::{CancellationToken, ToolError, ToolExecutionContext, ToolExecutor};

use super::database_tools::DatabaseTools;

#[async_trait]
impl ToolExecutor for DatabaseTools {
    /// Settles the retained read workers only. The read witnesses survive:
    /// a read anywhere earlier in this run admits a later write or edit,
    /// and the loop settles at pause boundaries and at the end of the
    /// run — clearing here would force a re-read after every pause.
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        self.workspace_tasks.settle().await;
        Ok(())
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute_with_context(
            name,
            arguments,
            ToolExecutionContext {
                result_cap: self.context_byte_budget,
                cancellation: CancellationToken::new(),
            },
        )
        .await
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute_with_context(
            name,
            arguments,
            ToolExecutionContext {
                result_cap,
                cancellation: CancellationToken::new(),
            },
        )
        .await
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        self.execute_read_only(name, arguments, context).await
    }
}
