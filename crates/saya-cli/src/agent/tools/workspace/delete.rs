use saya_agent::{
    LocalStateEffect, ToolConcurrency, ToolDefinition, ToolEffect, ToolError, ToolExecutionContext,
};
use saya_types::{Sha256Digest, WorkspacePath};
use serde_json::{Value, json};

pub(in crate::agent::tools) fn definition() -> ToolDefinition {
    ToolDefinition {
        name:"delete".into(),
        description:"Delete one regular file in this owned checkout using its exact digest from a fresh read. Refuses stale, missing, private, ignored and ungranted targets; never deletes directories or applies changes to the original.".into(),
        read_only:false,
        parameters:json!({"type":"object","properties":{
            "path":{"type":"string","maxLength":512},
            "expected_digest":{"type":"string","minLength":64,"maxLength":64,"pattern":"^[0-9a-f]{64}$"}},
            "required":["path","expected_digest"],"additionalProperties":false}),
        effect:ToolEffect {database_data:false,external_side_effect:false,requires_approval:true,local_state:LocalStateEffect::WriteWorkspace},
        concurrency:ToolConcurrency::Serial,
        completion:Some("workspace file deleted".into()),
    }
}
pub(in crate::agent::tools) fn refused(message: &'static str) -> ToolError {
    ToolError::NoEffect(Box::new(ToolError::Runner(message.into())))
}
pub(in crate::agent::tools) async fn execute(
    owner: &crate::commands::coding::checks::ChildCodingOperations,
    arguments: Value,
    context: ToolExecutionContext,
) -> Result<Value, ToolError> {
    super::validate_arguments("delete", &arguments)
        .map_err(|error| ToolError::NoEffect(Box::new(error)))?;
    let path = arguments["path"]
        .as_str()
        .ok_or_else(|| refused("deletion path must be a string"))?;
    // An absolute path under the root is the same file as its relative
    // spelling; anything outside is refused with the root named.
    let path = saya_harness::workspace::relative_argument(owner.application.root_path(), path)
        .map_err(|error| ToolError::NoEffect(Box::new(ToolError::Runner(error.to_string()))))?
        .into_owned();
    if path.len() > 512 {
        return Err(refused("deletion path exceeds its bound"));
    }
    let path =
        WorkspacePath::try_from(path).map_err(|_| refused("deletion path is not admissible"))?;
    let expected = arguments["expected_digest"]
        .as_str()
        .and_then(|text| Sha256Digest::parse(text).ok())
        .ok_or_else(|| refused("deletion requires a lowercase SHA-256 expected_digest"))?;
    owner
        .delete_workspace(path, expected, context.cancellation)
        .await
}
