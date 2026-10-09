// -- shared tool-body helpers ------------------------------------------------

/// The JSON-RPC envelope floor reserve (A922-5, D5): the response bound
/// applies to the final wire line, so a body that cannot see the request id
/// reserves this much on top of the serialized result for the response
/// object (`jsonrpc`, `id`, framing) that wraps it. The exact envelope for
/// the ACTUAL id is measured where the id is known: by the narrowing query
/// tool, and again at the dispatch gate — that exact measure also counts the
/// writer's newline and the `resultType` discriminator, which this floor
/// covers many times over.
pub(crate) const RESPONSE_ENVELOPE_HEADROOM_BYTES: usize = 4 * 1024;

/// The writer's framing byte (D9): the transport appends `\n` after every
/// serialized frame, so the reply line is one byte longer than the
/// serialized message. The transport is the single writer; this constant
/// mirrors it.
pub(crate) const RESPONSE_FRAME_NEWLINE_BYTES: usize = 1;

/// The serialized cost of the `resultType` discriminator (SEP-2322, D9): rmcp
/// 3.5.0 writes `"resultType":"complete"` into the result for peers that
/// negotiate 2026-07-28+ and strips it for legacy peers after this measure —
/// so the budget counts it ALWAYS: rmcp removing it can only shrink the
/// line. The value is the exact serialized cost of the field with its
/// separator, pinned by a unit test against rmcp's own serializer, never
/// estimated.
pub(crate) const RESULT_TYPE_DISCRIMINATOR_BYTES: usize = r#","resultType":"complete""#.len();

/// The exact JSON-RPC envelope bytes for one request id (D9): the wrapper
/// `{"jsonrpc":"2.0","id":<id>,"result":…}` — id escapes and non-ASCII
/// included, never estimated — measured from rmcp's own serialization around
/// a probe result. Newline-free: the writer's trailing newline is counted by
/// [`response_wire_bytes`]. An unserializable id measures as unbounded:
/// refused, never sent.
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
/// with the `resultType` discriminator budgeted as always present, plus the
/// exact JSON-RPC envelope for the actual request id, plus the writer's
/// trailing newline. An unserializable part measures as unbounded: refused,
/// never sent.
pub(crate) fn response_wire_bytes(result: &CallToolResult, id: &RequestId) -> usize {
    result_line_bytes(result).saturating_add(response_envelope_bytes(id))
}

/// The bytes one built result contributes to the reply line: its serialized
/// form plus the writer's newline, with an absent `resultType` discriminator
/// budgeted as present. Saturating: an unserializable result measures as
/// unbounded.
fn result_line_bytes(result: &CallToolResult) -> usize {
    serde_json::to_vec(result).map_or(usize::MAX, |bytes| {
        bytes
            .len()
            .saturating_add(if result.result_type.is_some() {
                0
            } else {
                RESULT_TYPE_DISCRIMINATOR_BYTES
            })
            .saturating_add(RESPONSE_FRAME_NEWLINE_BYTES)
    })
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
/// built result, the `resultType` discriminator, and the trailing newline —
/// before it leaves. Bodies without the id reserve only the floor, so this
/// is what keeps a large client-controlled id from pushing the reply line
/// over the bound. The replacement error is itself a short line for every id
/// the inbound line gate admits; for a hypothetical id so large that nothing
/// fits, the error is still the only correlated reply there is.
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
