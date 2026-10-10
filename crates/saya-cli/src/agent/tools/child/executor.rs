//! The child tool executor: admission immediately before the existing anchored mutation.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::policy::ChildAuthorityView;
use saya_types::{ManagedCheckoutId, WorkspacePath};

use super::super::DatabaseTools;
use super::super::workspace;
use super::{ChildToolsError, TrustedCheckoutPair};

/// A tool set bound to one child's verified managed checkout: the workspace
/// tools for the child root plus the narrowed authority that admits writes.
pub(crate) struct ChildTools {
    tools: DatabaseTools,
    view: ChildAuthorityView,
    checkout_id: ManagedCheckoutId,
    permit_writes: bool,
}

/// Builds the definitions and the executor for one child from one view: the
/// two stay in agreement because `permit` is computed once and used for both.
pub(crate) fn child_toolset(
    view: &ChildAuthorityView,
    pair: &TrustedCheckoutPair,
) -> Result<(Vec<ToolDefinition>, ChildTools), ChildToolsError> {
    // A view narrowed for another checkout is refused at build time.
    if view.managed_checkout_id() != Some(pair.checkout_id()) {
        return Err(ChildToolsError::CheckoutMismatch);
    }
    // Computed once: both the definitions and the executor must agree on it.
    let permit = view.managed_checkout_id().is_some() && !view.write_paths().is_empty();
    let mut definitions = Vec::new();
    workspace::append_definitions(&mut definitions, permit);
    let tools = DatabaseTools::for_child_checkout(Arc::clone(&pair.root));
    Ok((
        definitions,
        ChildTools {
            tools,
            view: view.clone(),
            checkout_id: pair.checkout_id().clone(),
            permit_writes: permit,
        },
    ))
}

/// Admission happens immediately before the existing anchored mutation; the
/// checkout id never comes from model arguments; data (SQL), process, scratch
/// and memory tools are absent: unknown names are unsupported.
#[async_trait]
impl ToolExecutor for ChildTools {
    async fn execute(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<serde_json::Value, ToolError> {
        match name {
            "workspace_write" | "workspace_edit" => {
                if !self.permit_writes {
                    return Err(ToolError::UnsupportedTool);
                }
                let path = arguments
                    .get("path")
                    .and_then(serde_json::Value::as_str)
                    .ok_or(ToolError::PathNotString)?;
                let workspace_path = WorkspacePath::try_from(path.to_owned()).map_err(|_| {
                    ToolError::WorkspaceWrite("the path is not an admissible workspace path".into())
                })?;
                self.view
                    .admit_workspace_write(&self.checkout_id, &workspace_path)
                    .map_err(|_| {
                        ToolError::WorkspaceWrite(
                            "the child authority does not admit this write".into(),
                        )
                    })?;
            }
            "workspace_read" | "workspace_list" | "glob" | "grep" => {}
            _ => return Err(ToolError::UnsupportedTool),
        }
        workspace::execute_tool(&self.tools, name, &arguments).await
    }
}
