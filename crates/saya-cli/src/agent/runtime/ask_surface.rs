//! The one-shot `ask` surface's composition: the database tools, the tools of
//! the servers the user configured, and the one `[tools] allow` gate in front
//! of both.
//!
//! `ask` is headless in the sense that matters to an MCP server: no operator
//! is composed for it, so the lane is built with no consent and every call is
//! refused with the typed reason naming `--approval-mode bypass`. The lane is
//! composed for the turn and dropped with it, so the servers' lifetime is the
//! surface's.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalPolicy, CancellationToken, ToolDefinition, ToolError, ToolExecutionContext,
    ToolExecutor,
};
use saya_config::ResolvedTools;

use crate::agent::tools::consent::Caller;
use crate::agent::tools::{DatabaseTools, NarrowedExecutor};
use crate::config::runtime::RuntimeConfig;
use crate::mcp::client::McpTools;

/// Starts the configured servers for one `ask` turn, or `None` where the
/// mode reaches none or the configuration names none.
pub(crate) async fn lane(
    runtime: &RuntimeConfig,
    policy: ApprovalPolicy,
    cancellation: &CancellationToken,
) -> Option<Arc<McpTools>> {
    McpTools::connect_async(
        runtime.resolved.mcp_servers.clone(),
        Arc::new(runtime.secret_resolver()),
        policy,
        None,
        Caller::Session,
        cancellation.clone(),
    )
    .await
}

/// The definitions the one-shot `ask` surface advertises: the database
/// surface, then the servers' tools, then the one `[tools] allow` list.
pub(crate) fn definitions(
    allow_query_data: bool,
    has_state_store: bool,
    permit_candidate_writes: bool,
    mcp: Option<&Arc<McpTools>>,
    tools: &ResolvedTools,
) -> Vec<ToolDefinition> {
    let mut definitions = DatabaseTools::definitions(
        allow_query_data,
        has_state_store,
        permit_candidate_writes,
        false,
    );
    if let Some(mcp) = mcp {
        definitions.extend(mcp.definitions());
    }
    crate::agent::tools::narrowed(definitions, tools)
}

/// The one-shot `ask` surface's executor: the database tools, the MCP lane
/// for the names it owns, and the one `[tools] allow` gate in front of both.
pub(crate) fn executor(
    database: &Arc<DatabaseTools>,
    mcp: Option<Arc<McpTools>>,
    tools: ResolvedTools,
) -> Arc<dyn ToolExecutor> {
    Arc::new(NarrowedExecutor::new(
        tools,
        Arc::new(AskTools {
            mcp,
            inner: Arc::clone(database) as Arc<dyn ToolExecutor>,
        }),
    ))
}

/// The surface's own composite: an `mcp__` name the lane owns goes to the
/// lane — whose consent rule decides it — and every other name keeps the
/// database surface's answer.
struct AskTools {
    mcp: Option<Arc<McpTools>>,
    inner: Arc<dyn ToolExecutor>,
}

impl AskTools {
    fn mcp_for(&self, name: &str) -> Option<&Arc<McpTools>> {
        self.mcp.as_ref().filter(|_| McpTools::owns(name))
    }
}

#[async_trait]
impl ToolExecutor for AskTools {
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        self.inner.settle_invocation().await
    }

    fn turn_context_note(&self) -> Option<String> {
        self.inner.turn_context_note()
    }

    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match self.mcp_for(name) {
            Some(mcp) => mcp.execute(name, arguments).await,
            None => self.inner.execute(name, arguments).await,
        }
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        match self.mcp_for(name) {
            Some(mcp) => {
                mcp.execute_with_result_cap(name, arguments, result_cap)
                    .await
            }
            None => {
                self.inner
                    .execute_with_result_cap(name, arguments, result_cap)
                    .await
            }
        }
    }

    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        match self.mcp_for(name) {
            Some(mcp) => mcp.execute_with_context(name, arguments, context).await,
            None => {
                self.inner
                    .execute_with_context(name, arguments, context)
                    .await
            }
        }
    }
}
