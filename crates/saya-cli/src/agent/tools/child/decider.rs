//! The factory-installed child approval: the harness coarse effect gate plus
//! exact per-call write admission, decided before execution so a refused
//! write carries no permit and settles no uncertain effect.

use async_trait::async_trait;
use saya_agent::{ApprovalDecider, ToolDefinition};
use saya_harness::coding::policy::{ChildApprovalDecider, ChildAuthorityView};
use saya_types::{ManagedCheckoutId, WorkspacePath};
use serde_json::Value;

const WRITE_TOOLS: [&str; 2] = ["workspace_write", "workspace_edit"];

/// Wraps the child's coarse decider with the one exact check the executor
/// would otherwise refuse mid-run: whether this call's path is an assigned
/// write path. Reading arguments only ever narrows — a model-supplied path
/// can deny a call but never widen authority — and the executor keeps the
/// same admission as defense in depth, so the two can only agree.
pub(crate) struct ExactWriteChildApproval {
    coarse: ChildApprovalDecider,
    view: ChildAuthorityView,
    checkout_id: ManagedCheckoutId,
}

impl ExactWriteChildApproval {
    pub(crate) fn new(view: &ChildAuthorityView, checkout_id: ManagedCheckoutId) -> Self {
        Self {
            coarse: ChildApprovalDecider::from_view(view),
            view: view.clone(),
            checkout_id,
        }
    }

    /// Exact admission of one write call; `Err` covers a missing or
    /// non-admissible path argument as well as an unassigned path, because
    /// the executor would refuse every one of these before mutating.
    fn write_admission(&self, arguments: &Value) -> Result<WorkspacePath, ()> {
        let path = arguments.get("path").and_then(Value::as_str).ok_or(())?;
        let path = WorkspacePath::try_from(path.to_owned()).map_err(|_| ())?;
        self.view
            .admit_workspace_write(&self.checkout_id, &path)
            .map_err(|_| ())?;
        Ok(path)
    }
}

#[async_trait]
impl ApprovalDecider for ExactWriteChildApproval {
    async fn approve(&self, tool: &ToolDefinition, arguments: &Value) -> bool {
        if !self.coarse.approve(tool, arguments).await {
            return false;
        }
        !WRITE_TOOLS.contains(&tool.name.as_str()) || self.write_admission(arguments).is_ok()
    }

    fn refusal_detail(&self, tool: &ToolDefinition, arguments: &Value) -> Option<String> {
        if WRITE_TOOLS.contains(&tool.name.as_str()) && self.write_admission(arguments).is_err() {
            return Some("the child authority does not admit this write".to_string());
        }
        self.coarse.refusal_detail(tool, arguments)
    }
}
