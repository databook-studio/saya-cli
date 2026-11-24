//! The job surface's tool executor: `CodingDatabaseTools` is not a
//! `ToolExecutor`, so this adapter is the one thing between the plugin contract
//! and the job's own `execute`.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{AgentLimits, CancellationToken, ToolError, ToolExecutionContext, ToolExecutor};
use saya_harness::coding::policy::ChildAuthorityView;

use super::DECLARED_TOOLS;
use crate::commands::coding::database::CodingDatabaseTools;

pub(super) struct JobToolExecutor {
    tools: Arc<CodingDatabaseTools>,
    view: Option<ChildAuthorityView>,
}

impl JobToolExecutor {
    pub(super) fn new(tools: Arc<CodingDatabaseTools>, view: Option<ChildAuthorityView>) -> Self {
        Self { tools, view }
    }

    /// Forwards to the job's own tools, which re-check profile admission and the
    /// read-only safety layer per call. A name the plugin never declared is
    /// refused here rather than handed on: the registry routes only declared
    /// names, so a call outside them is a bug on the caller's side.
    async fn serve(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        if !DECLARED_TOOLS.contains(&name) {
            return Err(ToolError::UnsupportedTool);
        }
        self.tools
            .execute(self.view.as_ref(), name, arguments, context)
            .await
    }
}

#[async_trait]
impl ToolExecutor for JobToolExecutor {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        self.serve(
            name,
            arguments,
            ToolExecutionContext {
                result_cap: AgentLimits::default().context_byte_budget,
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
        self.serve(
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
        self.serve(name, arguments, context).await
    }
}
