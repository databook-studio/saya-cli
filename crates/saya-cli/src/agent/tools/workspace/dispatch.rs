use saya_agent::ToolError;

use super::super::DatabaseTools;

pub(in crate::agent::tools) async fn execute(
    tools: &DatabaseTools,
    name: &str,
    arguments: &serde_json::Value,
) -> Result<serde_json::Value, ToolError> {
    match name {
        "workspace_read" => tools.workspace_read(arguments).await,
        "workspace_list" => tools.workspace_list(arguments).await,
        "workspace_write" => tools.workspace_write(arguments).await,
        "workspace_edit" => tools.workspace_edit(arguments).await,
        "glob" => tools.workspace_glob(arguments).await,
        "grep" => tools.workspace_grep(arguments).await,
        _ => Err(ToolError::UnsupportedTool),
    }
}
