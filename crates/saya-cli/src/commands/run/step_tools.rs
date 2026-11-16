//! One run step's composite: the run's `shell` by name, the configured
//! servers' tools by name, everything else through the shared `RunTools`.
//!
//! The wrapper exists so the run's command door and MCP lane ride the same
//! composition the coding root dispatches through, without teaching the
//! shared composite — which the interactive session also holds — about a tool
//! a run alone advertises.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolError, ToolExecutionContext, ToolExecutor};

use crate::agent::tools::RunTools;
use crate::agent::tools::shell_policy::{self, ShellTool};
use crate::mcp::client::McpTools;

pub(super) struct RunStepTools {
    pub(super) shell: Option<Arc<ShellTool>>,
    /// The run's MCP lane, shared by clone with every step. `None` means the
    /// run composed no lane, so no `mcp__` name is served here.
    pub(super) mcp: Option<Arc<McpTools>>,
    pub(super) inner: Arc<RunTools>,
    /// Whether this step's list offers the notebook edit: the run
    /// composition's own answer to whether the run's workspace holds a
    /// notebook, decided once for the whole run. The definitions the step
    /// advertises were narrowed by it, so the dispatch refuses the name
    /// exactly where the advertisement left it out.
    pub(super) notebook_edit: bool,
    /// What this step's goal states. `list` and the other optional tools are
    /// advertised only when it asks for them or the user named them.
    pub(super) evidence: crate::agent::tools::advertisement::Evidence,
    /// The run's `[tools] allow` set, so an explicit name re-advertises a
    /// default-suppressed tool.
    pub(super) tools: saya_config::ResolvedTools,
}

impl RunStepTools {
    /// The member that serves `name`, or `None` for the shared fall-through.
    fn shell_for(&self, name: &str) -> Option<&Arc<ShellTool>> {
        self.shell
            .as_ref()
            .filter(|_| shell_policy::is_shell_name(name))
    }

    /// The lane that serves `name`, where this run composed one and the name
    /// is an `mcp__` name. A name the lane does not actually hold is refused
    /// by the lane's own dispatch, exactly as it is on every other surface.
    fn mcp_for(&self, name: &str) -> Option<&Arc<McpTools>> {
        self.mcp.as_ref().filter(|_| McpTools::owns(name))
    }

    /// The `[tools] allow` gate for the one member this wrapper serves itself:
    /// the run's shell rides no composite, so it is checked here before the
    /// call is routed. Every other name is checked by the composite below.
    /// The notebook edit is checked here too: a run whose workspace holds no
    /// notebook composed the name out of this step's list, so a call to it is
    /// the unknown-tool refusal a member this step never carried gets.
    fn admits(&self, name: &str) -> bool {
        (self.notebook_edit || name != "notebook_edit")
            && self.inner.admits(name)
            && crate::agent::tools::advertisement::advertised(&self.evidence, &self.tools, name)
    }
}

#[async_trait]
impl ToolExecutor for RunStepTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        match (self.shell_for(name), self.mcp_for(name)) {
            (Some(shell), _) => shell.execute(name, arguments).await,
            (None, Some(mcp)) => mcp.execute(name, arguments).await,
            (None, None) => self.inner.execute(name, arguments).await,
        }
    }

    async fn execute_with_result_cap(
        &self,
        name: &str,
        arguments: serde_json::Value,
        result_cap: usize,
    ) -> Result<serde_json::Value, ToolError> {
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        match (self.shell_for(name), self.mcp_for(name)) {
            (Some(shell), _) => {
                shell
                    .execute_with_result_cap(name, arguments, result_cap)
                    .await
            }
            (None, Some(mcp)) => {
                mcp.execute_with_result_cap(name, arguments, result_cap)
                    .await
            }
            (None, None) => {
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
        if !self.admits(name) {
            return Err(ToolError::UnsupportedTool);
        }
        match (self.shell_for(name), self.mcp_for(name)) {
            (Some(shell), _) => shell.execute_with_context(name, arguments, context).await,
            (None, Some(mcp)) => mcp.execute_with_context(name, arguments, context).await,
            (None, None) => {
                self.inner
                    .execute_with_context(name, arguments, context)
                    .await
            }
        }
    }
}
