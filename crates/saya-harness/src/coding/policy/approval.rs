//! The child's approval decider.
//!
//! Constructible only from an already-narrowed [`ChildAuthorityView`].
//! External side effects and database data are always denied before anything
//! else; read-shaped effects pass; workspace writes and process runs pass
//! only when the view grants them and only for the tools that enforce exact
//! admission per call.
//! Call arguments are NEVER read, so a parent prompt or a model-supplied
//! "grant" cannot widen anything. There is no session-grant, journal or
//! prompt input: this is the coarse effect-level gate, and per-call exact
//! admission (path / program) stays in the executor.

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, LocalStateEffect, ToolDefinition, read_only_permits};

use super::ChildAuthorityView;

/// Coarse effect-level gate derived from the child's authority view.
#[derive(Debug)]
pub struct ChildApprovalDecider {
    workspace_writes: bool,
    commands: bool,
}

impl ChildApprovalDecider {
    /// Narrows to exactly what the already-narrowed view grants.
    pub fn from_view(view: &ChildAuthorityView) -> Self {
        let capabilities = view.capabilities();
        Self {
            workspace_writes: view.managed_checkout_id().is_some()
                && !view.write_paths().is_empty(),
            commands: capabilities.runner.is_some() || capabilities.interpreter.is_some(),
        }
    }
}

#[async_trait]
impl ApprovalDecider for ChildApprovalDecider {
    async fn approve(&self, tool: &ToolDefinition, _arguments: &serde_json::Value) -> bool {
        if tool.effect.external_side_effect || tool.effect.database_data {
            return false;
        }
        if read_only_permits(&tool.effect) {
            return true;
        }
        if matches!(tool.effect.local_state, LocalStateEffect::WriteWorkspace) {
            return match tool.name.as_str() {
                "workspace_write" | "workspace_edit" => self.workspace_writes,
                "run_program" => self.commands,
                _ => false,
            };
        }
        false
    }

    fn refusal_detail(
        &self,
        _tool: &ToolDefinition,
        _arguments: &serde_json::Value,
    ) -> Option<String> {
        Some("the child authority does not grant this tool".to_string())
    }
}
