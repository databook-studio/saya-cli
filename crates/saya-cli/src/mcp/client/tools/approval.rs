//! The consent rule: what a call to an MCP tool is asked about, and what the
//! model is told a server's tool is.
//!
//! One decision, three surfaces. [`McpTools::refusal`] states whether this
//! mode serves a call at all; `ApprovalDecider` puts the call to whoever the
//! surface has — the coding root's human [`Operator`], the interactive
//! session's own per-turn decider — and `OwnApproval` claims the `mcp__` names
//! so a surface that wraps its own decider routes exactly these calls here.
//! `ToolExecutor` is the last of the three: once a call is approved, this is
//! the value that runs it.
//!
//! [`definition`] is built in one place and read by two: the advertisement the
//! model sees, and the definition a session's decider is asked about. A second
//! construction would let the tool the model was offered differ from the tool
//! the operator was asked about.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{
    ApprovalDecider, ApprovalPolicy, LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect,
    ToolError, ToolExecutionContext, ToolExecutor,
};
use serde_json::Value;

use super::Consent;
use super::McpTools;
use crate::agent::tools::consent::{Caller, OwnApproval, Prompt};
use crate::mcp::client::server::McpTool;

/// The line the operator's prompt opens with, in this client's own words.
fn line(caller: &Caller, server: &str, tool: &str) -> String {
    format!(
        "{} (server {server:?}, tool {tool:?})",
        caller.line("Call an MCP tool", "call an MCP tool")
    )
}

/// One tool's advertised definition. Built in one place, so the
/// advertisement the model reads and the definition a session's decider is
/// asked about cannot drift.
pub(super) fn definition(tool: &McpTool) -> ToolDefinition {
    ToolDefinition {
        name: tool.exposed.clone(),
        description: tool.description.clone(),
        // A server's tool is never read-only in saya's sense: nothing here
        // bounds what the program does with the call.
        read_only: false,
        parameters: tool.parameters.clone(),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: true,
            requires_approval: true,
            local_state: LocalStateEffect::None,
        },
        // One server is one process with one pipe and one in-flight exchange,
        // so two of its calls cannot overlap whatever the loop would allow.
        concurrency: ToolConcurrency::Serial,
        completion: None,
    }
}

#[async_trait]
impl ApprovalDecider for McpTools {
    /// `bypass` calls, the asking modes put the call to the operator — the
    /// mode pre-answers no MCP tool — and the two no-server modes deny: the
    /// same decision [`McpTools::refusal`] states, so a denial always has a
    /// reason to give.
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        if self.refusal().is_some() {
            return false;
        }
        let Some((server, entry)) = self.entry(&tool.name) else {
            return false;
        };
        match self.policy {
            ApprovalPolicy::Bypass => true,
            ApprovalPolicy::Ask | ApprovalPolicy::AcceptEdits => match &self.consent {
                Consent::Operator(operator) => {
                    let prompt = Prompt::new(
                        Arc::clone(operator),
                        line(&self.caller, server.name(), &entry.tool),
                    );
                    prompt.ask(arguments).await
                }
                Consent::Decider(decider) => decider.approve(&definition(entry), arguments).await,
                Consent::Absent => false,
            },
            ApprovalPolicy::ReadOnly | ApprovalPolicy::Never => false,
        }
    }

    fn refusal_detail(&self, tool: &ToolDefinition, _arguments: &Value) -> Option<String> {
        if let Some(reason) = self.refusal() {
            return Some(reason.to_owned());
        }
        self.entry(&tool.name)
            .is_none()
            .then(|| format!("{} is not a tool of any connected MCP server", tool.name))
    }
}

#[async_trait]
impl OwnApproval for McpTools {
    /// The client owns every `mcp__<server>__<tool>` name and nothing else, so a
    /// surface that wraps its own decider routes exactly these calls here.
    fn owns(&self, name: &str) -> bool {
        Self::owns(name)
    }
}

#[async_trait]
impl ToolExecutor for McpTools {
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: Value,
        _context: ToolExecutionContext,
    ) -> Result<Value, ToolError> {
        self.call(name, &arguments).await
    }

    async fn execute(&self, name: &str, arguments: Value) -> Result<Value, ToolError> {
        self.call(name, &arguments).await
    }
}
