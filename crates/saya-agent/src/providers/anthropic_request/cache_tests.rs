use super::*;

// --- Prompt caching -----------------------------------------------------
//
// saya already parses `cache_read_input_tokens` / `cache_creation_input_tokens`
// from Anthropic responses, but never asked the API to build a cache. These
// tests pin the request-side breakpoints that make that gauge non-trivial.

fn tool_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.into(),
        description: "desc".into(),
        read_only: true,
        parameters: json!({"type": "object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: crate::ToolConcurrency::Serial,
        completion: None,
    }
}

/// Every `cache_control` object in a body, counted recursively. Anthropic
/// caps a request at four breakpoints; this is the invariant the body must
/// keep regardless of how many tools or system messages arrive.
fn cache_breakpoint_count(value: &Value) -> usize {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let here = if k == "cache_control" { 1 } else { 0 };
                here + cache_breakpoint_count(v)
            })
            .sum(),
        Value::Array(items) => items.iter().map(cache_breakpoint_count).sum(),
        _ => 0,
    }
}

/// `system` is sent as a single-element array of text blocks so it can carry
/// a `cache_control` breakpoint. The block's text must equal, byte-for-byte,
/// the string the old bare-string form produced (`system_prompts.join("\n\n")`)
/// — only the envelope changes, never the content.
#[test]
fn system_serializes_as_one_text_block_with_the_same_text_as_the_old_string() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![
            ChatMessage::text("system", "You are helpful."),
            ChatMessage::text("system", "And careful."),
            ChatMessage::text("user", "hi"),
        ],
        ..Default::default()
    };
    let body = build_body(request, 1024, None);

    let system = body["system"].as_array().expect("system is an array");
    assert_eq!(system.len(), 1, "a single system block: {body}");
    assert_eq!(system[0]["type"], "text");
    assert_eq!(system[0]["text"], "You are helpful.\n\nAnd careful.");
}

/// The system block carries an `ephemeral` cache breakpoint — the marker
/// that asks Anthropic to cache the stable system prefix.
#[test]
fn system_block_carries_an_ephemeral_cache_breakpoint() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("system", "You are helpful.")],
        ..Default::default()
    };
    let body = build_body(request, 1024, None);
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
}

/// Tools are cached by marking the **last** tool in the array — everything
/// from the start of the prompt up to and including that block is cached,
/// so the last tool is the highest-value stable breakpoint. No other tool
/// carries a marker (one breakpoint covers the whole tool list).
#[test]
fn only_the_last_tool_carries_a_cache_breakpoint() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("user", "hi")],
        tools: vec![tool_def("first"), tool_def("second"), tool_def("third")],
        ..Default::default()
    };
    let body = build_body(request, 1024, None);
    let tools = body["tools"].as_array().expect("tools");
    assert_eq!(tools.len(), 3);
    assert_eq!(tools[2]["cache_control"]["type"], "ephemeral");
    assert!(
        tools[0].get("cache_control").is_none(),
        "non-last tool must not carry a breakpoint"
    );
    assert!(
        tools[1].get("cache_control").is_none(),
        "non-last tool must not carry a breakpoint"
    );
}

/// A request with no tools still serializes validly and still caches the
/// system — the system breakpoint stands on its own.
#[test]
fn a_request_with_no_tools_still_caches_the_system() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("system", "You are helpful.")],
        tools: Vec::new(),
        ..Default::default()
    };
    let body = build_body(request, 1024, None);
    assert!(
        body.get("tools").is_none(),
        "no tools field when none were supplied: {body}"
    );
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
}

/// Anthropic caps a request at four `cache_control` breakpoints. With the
/// stable prefix marked (system + last tool) the body uses two; this test
/// guards the invariant so a future addition can't silently exceed it.
#[test]
fn breakpoint_count_never_exceeds_four() {
    let request = ChatRequest {
        model: "claude-3-5-sonnet".into(),
        messages: vec![ChatMessage::text("system", "You are helpful.")],
        tools: vec![tool_def("a"), tool_def("b"), tool_def("c"), tool_def("d")],
        ..Default::default()
    };
    let body = build_body(request, 1024, None);
    let count = cache_breakpoint_count(&body);
    assert!(
        count >= 1,
        "the stable prefix must be marked with at least one breakpoint, got {count}: {body}"
    );
    assert!(
        count <= 4,
        "Anthropic caps at 4 breakpoints; got {count}: {body}"
    );
}
