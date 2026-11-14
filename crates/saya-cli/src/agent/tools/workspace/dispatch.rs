use saya_agent::{ToolError, ToolExecutionContext};
use saya_harness::workspace::Workspace;
use serde_json::Value;

use super::super::DatabaseTools;
use super::names::{canonical_name, normalize_arguments};

/// An absolute `path` under the root is accepted as its relative spelling, so
/// the read witness, the checkpoint recorder and every containment check see
/// one form. Any other path is left as it is, so each tool refuses it with the
/// same typed error it always has.
fn relative_path(workspace: &Workspace, arguments: Value) -> Value {
    let Some(object) = arguments.as_object() else {
        return arguments;
    };
    let Some(path) = object.get("path").and_then(Value::as_str) else {
        return arguments;
    };
    let Ok(relative) = workspace.relative_argument(path) else {
        return arguments;
    };
    if relative == path {
        return arguments;
    }
    let mut mapped = object.clone();
    mapped.insert("path".into(), Value::String(relative.into_owned()));
    Value::Object(mapped)
}

pub(in crate::agent::tools) async fn execute_with_context(
    tools: &DatabaseTools,
    name: &str,
    arguments: &serde_json::Value,
    context: ToolExecutionContext,
) -> Result<serde_json::Value, ToolError> {
    let name = canonical_name(name);
    let arguments = normalize_arguments(name, arguments);
    let arguments = match tools.workspace.as_deref() {
        Some(workspace) => relative_path(workspace, arguments),
        None => arguments,
    };
    let effective_arguments = match name {
        "write" | "notebook_edit" => tools.with_read_witness(&arguments, false),
        "edit" => tools.with_read_witness(&arguments, true),
        _ => arguments,
    };
    super::validate_arguments(name, &effective_arguments)?;
    match name {
        "read" => tools.workspace_read(&effective_arguments, context).await,
        "list" => tools.workspace_list(&effective_arguments, context).await,
        "write" | "edit" | "notebook_edit" => {
            if context.cancellation.is_cancelled() {
                return Err(ToolError::NoEffect(Box::new(ToolError::Runner(
                    "workspace mutation was cancelled before dispatch".into(),
                ))));
            }
            match name {
                "write" => tools.workspace_write(&effective_arguments).await,
                "notebook_edit" => tools.workspace_notebook_edit(&effective_arguments).await,
                _ => {
                    tools
                        .workspace_edit(&effective_arguments, &context.cancellation)
                        .await
                }
            }
        }
        "glob" | "grep" => {
            super::execute_search(tools, name, &effective_arguments, context, Vec::new()).await
        }
        _ => Err(ToolError::UnsupportedTool),
    }
}
