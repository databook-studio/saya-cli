//! The child tool executor: admission immediately before the existing anchored mutation.

use std::sync::Arc;

use async_trait::async_trait;
use saya_agent::{LocalStateEffect, ToolDefinition, ToolError, ToolExecutor};
use saya_harness::coding::policy::ChildAuthorityView;
use saya_harness::runner::RunProgram;
use saya_types::{ManagedCheckoutId, WorkspacePath};

use super::super::DatabaseTools;
use super::super::workspace;
use super::{ChildToolsError, TrustedCheckoutPair};

/// A tool set bound to one child's verified managed checkout: the workspace
/// tools for the child root plus the narrowed authority that admits writes.
/// Process tools exist only when the composition root supplies an
/// already-confined runner AND the view's scope admits the program.
pub(crate) struct ChildTools {
    tools: DatabaseTools,
    view: ChildAuthorityView,
    checkout_id: ManagedCheckoutId,
    permit_writes: bool,
    runner: Option<Arc<RunProgram>>,
    coding: Option<Arc<crate::commands::coding::checks::ChildCodingOperations>>,
}

/// Builds the definitions and the executor for one child from one view: the
/// two stay in agreement because `permit` is computed once and used for both.
/// Write-shaped child tools always go through the child decider; the loop's
/// own `permit_workspace_writes` limit is projected from the view by the
/// composition root.
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
    require_decider_for_writes(&mut definitions);
    let tools = DatabaseTools::for_child_checkout(Arc::clone(&pair.root));
    Ok((
        definitions,
        ChildTools {
            tools,
            view: view.clone(),
            checkout_id: pair.checkout_id().clone(),
            permit_writes: permit,
            runner: None,
            coding: None,
        },
    ))
}

/// Write-shaped child tools always go through the child decider: every
/// definition whose effect is a workspace write is marked so the loop
/// consults the child's decider before running it; the loop's own
/// `permit_workspace_writes` limit is projected from the view by the
/// composition root.
fn require_decider_for_writes(definitions: &mut [ToolDefinition]) {
    for definition in definitions {
        if matches!(
            definition.effect.local_state,
            LocalStateEffect::WriteWorkspace
        ) {
            definition.effect.requires_approval = true;
        }
    }
}

/// Builds the workspace half with [`child_toolset`] and, when the view's
/// scope admits process work at all, adds the composition root's
/// already-confined runner: its definition is appended and the executor
/// routes `run_program` to it after admission. Refused when the view grants
/// neither a runner nor an interpreter scope, so the tool is never surfaced
/// without admission behind it. Write-shaped child tools always go through
/// the child decider; the loop's own `permit_workspace_writes` limit is
/// projected from the view by the composition root.
pub(crate) fn child_toolset_with_runner(
    view: &ChildAuthorityView,
    pair: &TrustedCheckoutPair,
    runner: Arc<RunProgram>,
) -> Result<(Vec<ToolDefinition>, ChildTools), ChildToolsError> {
    let (mut definitions, mut tools) = child_toolset(view, pair)?;
    if !(view.capabilities().runner.is_some() || view.capabilities().interpreter.is_some()) {
        return Err(ChildToolsError::RunnerNotAdmitted);
    }
    definitions.push(runner.definition());
    require_decider_for_writes(&mut definitions);
    tools.runner = Some(runner);
    Ok((definitions, tools))
}

impl ChildTools {
    pub(crate) fn attach_coding(
        &mut self,
        coding: Arc<crate::commands::coding::checks::ChildCodingOperations>,
    ) {
        self.coding = Some(coding);
    }
}

/// Admission happens immediately before the existing anchored mutation; the
/// checkout id never comes from model arguments; data (SQL), scratch and
/// memory tools are absent: unknown names are unsupported. Process tools
/// exist only when the composition root supplies an already-confined runner
/// AND the view's scope admits the program: `run_program` routes to that
/// runner after admission, and `run_command` is never routed.
#[async_trait]
impl ToolExecutor for ChildTools {
    async fn execute_with_context(
        &self,
        name: &str,
        arguments: serde_json::Value,
        context: saya_agent::ToolExecutionContext,
    ) -> Result<serde_json::Value, ToolError> {
        if name.starts_with("coding_") {
            let coding = self.coding.as_ref().ok_or(ToolError::UnsupportedTool)?;
            return super::super::coding::child::execute(
                coding,
                name,
                arguments,
                &context.cancellation,
            )
            .await;
        }
        self.execute(name, arguments).await
    }
    async fn settle_invocation(&self) -> Result<(), ToolError> {
        if let Some(coding) = &self.coding {
            coding
                .settle()
                .await
                .map_err(|error| ToolError::Runner(error.to_string()))?;
        }
        Ok(())
    }

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
            "run_program" => {
                let Some(runner) = self.runner.as_ref() else {
                    return Err(ToolError::UnsupportedTool);
                };
                let program = arguments
                    .get("program")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| ToolError::Runner("the program is not a string".into()))?;
                let admitted = if saya_types::is_refused_runner_program(program) {
                    self.view.admit_interpreter_program(program)
                } else {
                    self.view.admit_runner_program(program)
                };
                admitted.map_err(|_| {
                    ToolError::Runner("the child authority does not admit this program".into())
                })?;
                return runner.execute(name, arguments).await;
            }
            _ => return Err(ToolError::UnsupportedTool),
        }
        workspace::execute_tool(&self.tools, name, &arguments).await
    }
}
