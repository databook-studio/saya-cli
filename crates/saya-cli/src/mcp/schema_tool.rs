//! The `schema` tool (task Db): live discovery with the state-store cache
//! fallback — the same `agent::state_tools::schema` operation the agent's
//! schema tool uses, over the same connector factory `saya query` uses. The
//! compact tree is the answer the CLI's schema surface renders, wrapped with
//! the profile name; nothing about hosts, paths, or identities is in it.

use rmcp::model::{CallToolRequestParams, CallToolResponse};
use serde_json::Value;

use super::{connector::build_connector, context::McpContext, policy::ServePolicy, tools};

pub(crate) async fn schema(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
) -> Result<CallToolResponse, rmcp::ErrorData> {
    let name = tools::required_string(request, "profile")?;
    let (profile, identity) = match context.allowed_profile(policy.allowlist(), name) {
        Ok(pair) => pair,
        Err(message) => return Ok(tools::error_result(message)),
    };
    let connector = match build_connector(&profile, context).await {
        Ok(connector) => connector,
        Err(message) => return Ok(tools::error_result(message)),
    };
    if let Err(error) = connector.connect().await {
        return Ok(tools::error_result(error.to_string()));
    }
    let compact = crate::agent::state_tools::schema(
        connector.as_ref(),
        Some(&context.store),
        Some(identity.as_str()),
    )
    .await;
    match compact {
        Ok(mut payload) => {
            if let Some(object) = payload.as_object_mut() {
                object.insert("profile".into(), Value::String(name.to_owned()));
            }
            tools::bounded_result(policy, payload)
        }
        Err(error) => Ok(tools::error_result(error.to_string())),
    }
}
