//! The MCP tool dispatch (tasks Da + Db): the single place a call is
//! admitted or refused before a body runs. Every tool body lives beside its
//! concern — [`super::schema_tool`] and [`super::query_tool`],
//! [`super::context_tools`] (contracts), [`super::replay_tools`]
//! (investigation_run) — and shares the helpers here: the startup allowlist
//! gate, the data-sharing gate, the sanitized error shape, and the response
//! bound.

use rmcp::{
    ErrorData, RoleServer,
    model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode},
    service::RequestContext,
};
use serde_json::json;

use super::{
    catalog, context::McpContext, context_tools, policy::ServePolicy, query_tool, replay_tools,
    schema_tool,
};

/// Run one tool body. The name dispatch is the single extension point; an
/// unknown name is a method error, per the MCP spec. A row-returning tool
/// called while data sharing is off is refused as a tool-level error with
/// the gate's own words (invariant 2).
pub(crate) async fn run(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
    request_context: &RequestContext<RoleServer>,
) -> Result<CallToolResponse, ErrorData> {
    if catalog::is_row_returning(&request.name) && !policy.allow_data_sharing() {
        return Ok(error_result(format!(
            "{} is not available: data sharing is off for this server; restart \
             with --allow-data-sharing or set [ai] allow_data_sharing",
            request.name
        )));
    }
    match request.name.as_ref() {
        "contracts" => context_tools::contracts(policy, context, request).await,
        "investigation_run" => replay_tools::investigation_run(policy, context, request).await,
        "list_profiles" => run_list_profiles(policy, request).await,
        "query" => query_tool::query(policy, context, request, request_context).await,
        "schema" => schema_tool::schema(policy, context, request).await,
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
    bounded_result(policy, profiles_payload(policy))
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

// -- shared tool-body helpers ------------------------------------------------

/// One tool-level error result: the caller sees sanitized text, never
/// connection strings, paths, hosts, or secrets (invariant 5).
pub(crate) fn error_result(message: impl Into<String>) -> CallToolResponse {
    CallToolResult::error(vec![ContentBlock::text(sanitized(&message.into()))]).into()
}

/// Every text leaving a tool passes both scrubs: the secret-shaped redaction
/// and the terminal-control-character strip.
pub(crate) fn sanitized(text: &str) -> String {
    crate::render::sanitize_terminal(&saya_types::redact(text))
}

/// One successful structured result, bounded by the response limit; an
/// oversized payload is refused, never sent.
pub(crate) fn bounded_result(
    policy: &ServePolicy,
    payload: serde_json::Value,
) -> Result<CallToolResponse, ErrorData> {
    let bytes = serde_json::to_vec(&payload)
        .map_err(|_| ErrorData::internal_error("tool response is not serializable", None))?;
    if !policy.response_allowed(bytes.len()) {
        return Ok(error_result(format!(
            "the response exceeds the {}-byte response bound",
            super::policy::MAX_RESPONSE_BYTES
        )));
    }
    Ok(CallToolResult::structured(payload).into())
}

/// Reads a required string argument.
pub(crate) fn required_string<'a>(
    request: &'a CallToolRequestParams,
    key: &str,
) -> Result<&'a str, ErrorData> {
    string_arg(request, key)?
        .ok_or_else(|| ErrorData::invalid_params(format!("{key} is required"), None))
}

/// Reads an optional string argument; a non-string value is a parameter error.
pub(crate) fn optional_string<'a>(
    request: &'a CallToolRequestParams,
    key: &str,
) -> Result<Option<&'a str>, ErrorData> {
    string_arg(request, key)
}

fn string_arg<'a>(
    request: &'a CallToolRequestParams,
    key: &str,
) -> Result<Option<&'a str>, ErrorData> {
    let Some(arguments) = request.arguments.as_ref() else {
        return Ok(None);
    };
    match arguments.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(value)) => Ok(Some(value.as_str())),
        Some(_) => Err(ErrorData::invalid_params(
            format!("{key} must be a string"),
            None,
        )),
    }
}
