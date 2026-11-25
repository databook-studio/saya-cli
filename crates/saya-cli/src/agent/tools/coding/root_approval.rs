//! The root's approval decision, kept beside the surface it decides for.
//!
//! A call is decided by whichever lane owns it — the plugin's database lane
//! before the variant split, then the coordinator's advertised definitions, the
//! root shell, the web lane and the MCP lane each by their own mode — and every
//! other call by the exact-write decider under the job's mode. One impl, over
//! `RootTools`, so no caller can decide a call twice or differently.

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ImmutableAuthorityRefusal, ToolDefinition};
use serde_json::Value;

use super::direct::RootTools;
use super::direct_tools::DirectCodingTools;
use super::routes::routes_to_root_owned;
use crate::agent::tools::write_ask::{self, hidden_write};

#[async_trait]
impl ApprovalDecider for RootTools {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        // A chat-surface run decides its database tools in the plugin's lane,
        // before the variant split: the read-only surface allows every name it
        // advertises unasked, so the lane has to come first to be the ask it is.
        if let Some(allowed) = self.db_lane_approve(tool, arguments).await {
            return allowed;
        }
        match self {
            RootTools::ReadOnly(read_only) => read_only.approve(tool, arguments).await,
            RootTools::WorkingCheckout(direct) => {
                // Coordinator-routed tools are approved against the advertised
                // definitions; hidden first-ship names are not advertised. The
                // exact-write decider handles workspace tools, and the root
                // shell and the fetch tool each decide a call the way their own
                // mode says.
                if routes_to_root_owned(&tool.name) {
                    return direct.approves_coordinator(tool);
                }
                if DirectCodingTools::is_shell(&tool.name) {
                    return match &direct.shell {
                        Some(shell) => shell.approve(tool, arguments).await,
                        None => false,
                    };
                }
                if crate::agent::tools::web::is_web_tool(&tool.name) {
                    return match &direct.web {
                        Some(web) => web.approve(tool, arguments).await,
                        None => false,
                    };
                }
                // An MCP tool is a call to a program saya cannot bound, so it
                // is decided the way the root shell is: by its own lane.
                if crate::mcp::client::McpTools::owns(&tool.name) {
                    return match &direct.mcp {
                        Some(mcp) => mcp.approve(tool, arguments).await,
                        None => false,
                    };
                }
                // The exact-write admission decides what this call may
                // touch, and the job's mode then decides whether its consent
                // covers a write: only an `ask` job with an operator asks. A
                // write this surface does not offer at all is refused before
                // the question, because the executor would refuse it too.
                let workspace = direct
                    .workspace
                    .workspace()
                    .map(|workspace| workspace.root());
                match write_ask::for_root(&direct.coordinator.job, workspace) {
                    Some(_) if hidden_write(tool, |name| direct.advertises(name)) => false,
                    Some(ask) => ask.approve(&direct.decider, tool, arguments).await,
                    None => direct.decider.approve(tool, arguments).await,
                }
            }
        }
    }

    fn would_ask(&self, tool: &ToolDefinition, arguments: &Value) -> Option<bool> {
        // The lane's ask is the ask a `PermissionRequest` hook may pre-empt, so
        // the gate reads the same table the call is decided by.
        if let Some(asked) = self.db_lane_would_ask(tool, arguments) {
            return Some(asked);
        }
        match self {
            RootTools::ReadOnly(read_only) => read_only.would_ask(tool, arguments),
            RootTools::WorkingCheckout(direct) => {
                // A coordinator-routed tool is approved by its advertised
                // definition, not by an operator: no ask to pre-empt.
                if routes_to_root_owned(&tool.name) {
                    return Some(false);
                }
                let workspace = direct
                    .workspace
                    .workspace()
                    .map(|workspace| workspace.root());
                match write_ask::for_root(&direct.coordinator.job, workspace) {
                    Some(_) if hidden_write(tool, |name| direct.advertises(name)) => Some(false),
                    Some(ask) => ask.would_ask(direct.decider_for(tool), tool, arguments),
                    None => direct.decider_for(tool).would_ask(tool, arguments),
                }
            }
        }
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        match self {
            RootTools::ReadOnly(read_only) => read_only.refusal_detail(tool, arguments),
            RootTools::WorkingCheckout(direct) => {
                direct.decider_for(tool).refusal_detail(tool, arguments)
            }
        }
    }

    fn immutable_authority_refusal(
        &self,
        tool: &ToolDefinition,
        arguments: &Value,
    ) -> Option<ImmutableAuthorityRefusal> {
        match self {
            RootTools::ReadOnly(read_only) => {
                read_only.immutable_authority_refusal(tool, arguments)
            }
            RootTools::WorkingCheckout(direct) => direct
                .decider_for(tool)
                .immutable_authority_refusal(tool, arguments),
        }
    }
}
