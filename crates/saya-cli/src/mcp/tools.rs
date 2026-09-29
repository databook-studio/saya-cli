//! The MCP tool catalog (task Da skeleton): the advertised definition and
//! the body of `list_profiles`. The data tools of the database wiring
//! (task Db) add their definitions to [`advertised`] and their bodies beside
//! [`run_list_profiles`], behind the same policy hooks.

use std::sync::Arc;

use rmcp::{
    ErrorData,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode, Tool,
        ToolAnnotations, object,
    },
};
use serde_json::json;

use super::policy::{MAX_RESPONSE_BYTES, ServePolicy};

/// The tools `tools/list` advertises — the whole toolset a client can ever
/// call, fixed at startup.
pub(crate) fn advertised() -> Vec<Tool> {
    vec![list_profiles_tool()]
}

/// Run one tool body. The name dispatch is the single extension point the
/// data tools join; an unknown name is a method error, per the MCP spec.
pub(crate) async fn run(
    policy: &ServePolicy,
    request: &CallToolRequestParams,
) -> Result<CallToolResponse, ErrorData> {
    match request.name.as_ref() {
        "list_profiles" => run_list_profiles(policy, request).await,
        other => Err(ErrorData::new(
            ErrorCode::METHOD_NOT_FOUND,
            format!("unknown tool: {other}"),
            None,
        )),
    }
}

async fn run_list_profiles(
    policy: &ServePolicy,
    request: &CallToolRequestParams,
) -> Result<CallToolResponse, ErrorData> {
    if let Some(arguments) = request.arguments.as_ref()
        && !arguments.is_empty()
    {
        return Err(ErrorData::invalid_params(
            "list_profiles takes no arguments",
            None,
        ));
    }
    let payload = profiles_payload(policy);
    let bytes = serde_json::to_vec(&payload)
        .map_err(|_| ErrorData::internal_error("tool response is not serializable", None))?;
    if !policy.response_allowed(bytes.len()) {
        return Ok(CallToolResult::error(vec![ContentBlock::text(format!(
            "the response exceeds the {}-byte response bound",
            MAX_RESPONSE_BYTES
        ))])
        .into());
    }
    Ok(CallToolResult::structured(payload).into())
}

/// The `list_profiles` answer: the startup allowlist, names and dialects
/// only — the same set `tools/list` advertises, and the only set a client
/// can ever reach.
pub(crate) fn profiles_payload(policy: &ServePolicy) -> serde_json::Value {
    json!({
        "profiles": policy
            .allowlist()
            .iter()
            .map(|profile| json!({"name": profile.name, "dialect": profile.dialect}))
            .collect::<Vec<_>>(),
    })
}

fn list_profiles_tool() -> Tool {
    Tool::new(
        "list_profiles",
        "List the connection profiles this server may use, with their SQL \
         dialects; never paths, hosts, or identities.",
        Arc::new(object(json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false,
        }))),
    )
    .with_title("List profiles")
    .with_annotations(ToolAnnotations::new().read_only(true))
}
