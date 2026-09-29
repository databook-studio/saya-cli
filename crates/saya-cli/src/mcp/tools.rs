//! The MCP tool dispatch (tasks Da + Db): the single place a call is
//! admitted or refused before a body runs. Every tool body lives beside its
//! concern — [`super::schema_tool`] and [`super::query_tool`],
//! [`super::context_tools`] (contracts), [`super::replay_tools`]
//! (investigation_run) — and shares the helpers here: the startup allowlist
//! gate, the data-sharing gate, the sanitized error shape, and the response
//! bound.

use rmcp::{
    ErrorData, RoleServer,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ErrorCode,
        RequestId, ServerResult,
    },
    service::{RequestContext, TxJsonRpcMessage},
};
use serde_json::json;

use super::{
    catalog, context::McpContext, context_tools, policy::ServePolicy, query_tool, replay_tools,
    schema_tool,
};

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tools_tests;

/// Run one tool body. The name dispatch is the single extension point; an
/// unknown name is a method error, per the MCP spec. A row-returning tool
/// called while data sharing is off is refused as a tool-level error with
/// the gate's own words (invariant 2). Every complete response passes the
/// dispatch gate before it leaves.
pub(crate) async fn run(
    policy: &ServePolicy,
    context: &McpContext,
    request: &CallToolRequestParams,
    request_context: &RequestContext<RoleServer>,
) -> Result<CallToolResponse, ErrorData> {
    let response = if catalog::is_row_returning(&request.name) && !policy.allow_data_sharing() {
        Ok(error_result(format!(
            "{} is not available: data sharing is off for this server; restart \
             with --allow-data-sharing or set [ai] allow_data_sharing",
            request.name
        )))
    } else {
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
    };
    // The reply line carries the JSON-RPC envelope with the client's actual
    // request id; the dispatch gate measures it exactly before anything
    // leaves. A protocol error keeps its own (short) envelope.
    response.map(|response| bounded_response(policy, response, &request_context.id))
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

/// The JSON-RPC envelope floor reserve (A922-5, D5): the response bound
/// applies to the final wire line, so a body that cannot see the request id
/// reserves this much on top of the serialized result for the response
/// object (`jsonrpc`, `id`, framing) that wraps it. The exact envelope for
/// the ACTUAL id is measured where the id is known: by the narrowing query
/// tool, and again at the dispatch gate.
pub(crate) const RESPONSE_ENVELOPE_HEADROOM_BYTES: usize = 4 * 1024;

/// The exact JSON-RPC envelope bytes for one request id (D9): the wrapper
/// `{"jsonrpc":"2.0","id":<id>,"result":…}` plus the writer's trailing
/// newline, measured from rmcp's own serialization around a probe result —
/// id escapes and non-ASCII included, never estimated. An unserializable id
/// measures as unbounded: refused, never sent.
pub(crate) fn response_envelope_bytes(id: &RequestId) -> usize {
    let probe = ServerResult::CallToolResult(CallToolResult::structured(json!({})));
    let wrapped = serde_json::to_vec(&TxJsonRpcMessage::<RoleServer>::response(
        probe.clone(),
        id.clone(),
    ));
    let alone = serde_json::to_vec(&probe);
    match (wrapped, alone) {
        (Ok(wrapped), Ok(alone)) => wrapped.len().saturating_sub(alone.len()),
        _ => usize::MAX,
    }
}

/// The wire size of one reply line (A922-5, D5; D9): the final serialized
/// `CallToolResult` — rmcp's `structured` duplicates the payload into a text
/// content block, so the whole result is measured, never the payload alone —
/// plus the exact JSON-RPC envelope for the actual request id. An
/// unserializable part measures as unbounded: refused, never sent.
pub(crate) fn response_wire_bytes(result: &CallToolResult, id: &RequestId) -> usize {
    serde_json::to_vec(result)
        .map_or(usize::MAX, |bytes| bytes.len())
        .saturating_add(response_envelope_bytes(id))
}

/// The id-less wire size (A922-5, D5): the serialized result plus the floor
/// reserve — the conservative measure a body uses when the dispatch gate
/// will apply the exact envelope for the actual id.
pub(crate) fn result_wire_bytes(result: &CallToolResult) -> usize {
    serde_json::to_vec(result)
        .map_or(usize::MAX, |bytes| bytes.len())
        .saturating_add(RESPONSE_ENVELOPE_HEADROOM_BYTES)
}

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

/// The over-bound refusal: a tool-level error naming the bound, never the
/// payload.
fn over_bound() -> CallToolResponse {
    error_result(format!(
        "the response exceeds the {}-byte response bound",
        super::policy::MAX_RESPONSE_BYTES
    ))
}

/// One successful structured result, bounded by the response limit; the
/// measure is the built result — rmcp's `structured` duplicates the payload
/// into a text block — plus the floor envelope reserve (A922-5, D5); an
/// oversized result is refused, never sent. Bodies without the request id
/// bound against the floor here; the exact envelope for the ACTUAL id is
/// applied again at the dispatch gate.
pub(crate) fn bounded_result(
    policy: &ServePolicy,
    payload: serde_json::Value,
) -> Result<CallToolResponse, ErrorData> {
    let built = CallToolResult::structured(payload);
    if !policy.response_allowed(result_wire_bytes(&built)) {
        return Ok(over_bound());
    }
    Ok(built.into())
}

/// The bounded send for an already-built result (A922-5, D5; D9): tools that
/// narrow (the query tool) build the result themselves and measure it whole
/// against the exact envelope of the actual request id before sending here.
pub(crate) fn bounded_built_result(
    policy: &ServePolicy,
    result: CallToolResult,
    id: &RequestId,
) -> Result<CallToolResponse, ErrorData> {
    if !policy.response_allowed(response_wire_bytes(&result, id)) {
        return Ok(over_bound());
    }
    Ok(result.into())
}

/// The dispatch gate (D9): every complete response passes the exact wire
/// measure — the JSON-RPC envelope with the ACTUAL request id around the
/// built result, plus the trailing newline — before it leaves. Bodies
/// without the id reserve only the floor, so this is what keeps a large
/// client-controlled id from pushing the reply line over the bound. The
/// replacement error is itself a short line for every id the inbound line
/// gate admits; for a hypothetical id so large that nothing fits, the error
/// is still the only correlated reply there is.
fn bounded_response(
    policy: &ServePolicy,
    response: CallToolResponse,
    id: &RequestId,
) -> CallToolResponse {
    match response {
        CallToolResponse::Complete(result)
            if !policy.response_allowed(response_wire_bytes(&result, id)) =>
        {
            over_bound()
        }
        other => other,
    }
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

/// Reads an optional `{ name: string }` map as the canonical `name=value`
/// strings the run command binds (deliverable Dc-1: `investigation_run`
/// params). Absent or empty binds nothing; a non-object map, or a non-string
/// value in it, is a parameter error — the values themselves are only ever
/// handed to the same typed parser the CLI uses and are never echoed.
pub(crate) fn optional_string_map(
    request: &CallToolRequestParams,
    key: &str,
) -> Result<Vec<String>, ErrorData> {
    let Some(arguments) = request.arguments.as_ref() else {
        return Ok(Vec::new());
    };
    let Some(value) = arguments.get(key) else {
        return Ok(Vec::new());
    };
    let Some(map) = value.as_object() else {
        return Err(ErrorData::invalid_params(
            format!("{key} must be an object of string values"),
            None,
        ));
    };
    map.iter()
        .map(|(name, value)| {
            value.as_str().map_or_else(
                || {
                    Err(ErrorData::invalid_params(
                        format!("{key}.{name} must be a string"),
                        None,
                    ))
                },
                |text| Ok(format!("{name}={text}")),
            )
        })
        .collect()
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
