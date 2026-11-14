//! A tool result cut by the budget trim must stay readable: a valid-JSON
//! envelope when the result was JSON, the prefix-plus-marker form otherwise.
//! Cutting a JSON document at a byte offset and appending the marker hands the
//! model a string cut mid-way — invalid JSON the provider rejects.

use super::super::*;
use super::compaction::group;
use crate::ChatMessage;

const PROTECTED: usize = 2;
const NOTE: &str = "tool result exceeded the conversation byte budget";

/// The system/user prefix, one assistant tool call and its `tool` result.
fn conversation(result: String) -> Vec<ChatMessage> {
    vec![
        ChatMessage::text("system", "sys"),
        ChatMessage::text("user", "ask"),
        ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![crate::ToolCall {
                id: "call-newest".into(),
                name: "read_file".into(),
                arguments: serde_json::json!({"path": "a.rs"}),
            }],
            tool_call_id: None,

            images: Vec::new(),
            reasoning: None,
            reasoning_signature: None,
        },
        ChatMessage {
            role: "tool".into(),
            content: result,
            tool_calls: Vec::new(),
            tool_call_id: Some("call-newest".into()),

            images: Vec::new(),
            reasoning: None,
            reasoning_signature: None,
        },
    ]
}

/// A JSON tool result of the shape the harness emits: a long payload field
/// beside the metadata the model needs to place it.
fn json_result(inner: &str) -> String {
    serde_json::json!({"content": inner, "path": "a.rs"}).to_string()
}

fn trim(result: String, budget: usize) -> Vec<ChatMessage> {
    let mut messages = conversation(result);
    trim_to_budget(
        &mut messages,
        ContextBudget::new(budget, None),
        PROTECTED,
        None,
        &Default::default(),
    );
    messages
}

fn total(messages: &[ChatMessage]) -> usize {
    messages.iter().map(message_size).sum()
}

/// The bytes of the conversation the trim cannot shed: everything but the
/// result it is free to cut.
fn overhead(result: &str) -> usize {
    let messages = conversation(result.to_owned());
    total(&messages) - messages[3].content.len()
}

fn reply(messages: &[ChatMessage]) -> &ChatMessage {
    messages.last().expect("the newest group is kept")
}

fn parse(messages: &[ChatMessage]) -> serde_json::Value {
    serde_json::from_str(&reply(messages).content)
        .expect("a truncated JSON tool result must stay valid JSON")
}

/// The bug: a JSON result cut at a byte offset plus the marker is invalid
/// JSON. The truncated message must parse and carry what was lost.
#[test]
fn a_truncated_json_tool_result_parses_and_reports_the_original_size() {
    let original = json_result(&"x".repeat(8 * 1024));
    let messages = trim(original.clone(), 4 * 1024);
    assert!(
        reply(&messages).content.len() < original.len(),
        "the result must actually be cut"
    );
    let parsed = parse(&messages);
    assert_eq!(parsed["truncated"], serde_json::json!(true));
    assert_eq!(parsed["note"], serde_json::json!(NOTE));
    assert_eq!(
        parsed["original_bytes"],
        serde_json::json!(original.len()),
        "the model must be told how much was lost"
    );
    assert!(
        parsed["partial"].as_str().is_some_and(|s| !s.is_empty()),
        "the prefix that survived must be carried as a JSON string"
    );
}

/// The size bound holds for every target: the envelope is sized to the room
/// the trim computed, never over it — and it narrows rather than overrunning
/// the target, from the note and original size down to the flag alone.
#[test]
fn a_truncated_json_tool_result_fits_the_budget_for_every_target() {
    let original = json_result(&"x".repeat(32 * 1024));
    // The trim cannot shed the conversation around the result, so a target
    // with room for the smallest envelope — the 18-byte flag alone — is what
    // the bound is measured against.
    let floor = overhead(&original) + 18;
    for budget in [60, 90, 140, 260, 1_500, 2_048, 4_096, 16_384] {
        let messages = trim(original.clone(), budget);
        assert!(
            total(&messages) <= budget.max(floor),
            "budget {budget}: {} bytes kept",
            total(&messages)
        );
        let parsed = parse(&messages);
        assert_eq!(parsed["truncated"], serde_json::json!(true));
        if let Some(partial) = parsed["partial"].as_str() {
            assert!(original.starts_with(partial));
        }
    }
    // Escaping expands: `"` and `\` cost two bytes each, so the partial's
    // escaped length — not its raw length — is what has to fit.
    for budget in [1_500, 2_048, 4_096] {
        let messages = trim(json_result(&"\"\\".repeat(8 * 1024)), budget);
        assert!(
            total(&messages) <= budget,
            "escaped budget {budget}: {} bytes kept",
            total(&messages)
        );
        let parsed = parse(&messages);
        assert_eq!(parsed["truncated"], serde_json::json!(true));
    }
}

/// A target below even the smallest envelope has no room for one. The flag
/// alone then stands: still valid JSON, never a panic and never an overflow,
/// and over the target by at most the flag's own size — exactly as the marker
/// form stands when the marker itself does not fit.
#[test]
fn a_target_below_the_smallest_envelope_still_yields_valid_json() {
    let huge = "x".repeat(8 * 1024);
    let mut messages = vec![
        ChatMessage::text("system", "sys"),
        ChatMessage::text("user", "ask"),
    ];
    messages.extend(group(
        "call-newest",
        &huge,
        serde_json::json!({"content": "y".repeat(8 * 1024)}),
    ));
    // The overhead the trim cannot shed, plus one byte of room for the result.
    let overhead = total(&messages) - messages[3].content.len();
    let budget = overhead + 1;
    trim_to_budget(
        &mut messages,
        ContextBudget::new(budget, None),
        PROTECTED,
        None,
        &Default::default(),
    );
    let parsed = parse(&messages);
    assert_eq!(parsed["truncated"], serde_json::json!(true));
    assert_eq!(reply(&messages).content, "{\"truncated\":true}");
    assert!(total(&messages) <= budget + reply(&messages).content.len());
}

/// The envelope narrows in a fixed order as the target shrinks — the note
/// first, then the original size, then everything but the flag — so a target
/// too small for the full keys still gets a truthful, valid-JSON result.
#[test]
fn a_narrow_target_narrows_the_envelope_in_a_fixed_order() {
    let original = json_result(&"x".repeat(32 * 1024));
    let keys = |budget: usize| -> Vec<String> {
        parse(&trim(original.clone(), budget))
            .as_object()
            .expect("an envelope object")
            .keys()
            .cloned()
            .collect()
    };
    let wide = keys(4_000);
    let lean = keys(overhead(&original) + 80);
    let flag = keys(overhead(&original) + 20);
    assert!(wide.contains(&"note".to_owned()) && wide.contains(&"original_bytes".to_owned()));
    assert!(!lean.contains(&"note".to_owned()) && lean.contains(&"original_bytes".to_owned()));
    assert_eq!(flag, vec!["truncated".to_owned()]);
}

/// The cut lands on a character boundary: a multi-byte character is never
/// split, and the surviving prefix is a genuine prefix of the original text.
#[test]
fn a_truncated_json_tool_result_never_splits_a_character() {
    let original = json_result(&"é😀".repeat(2 * 1024));
    for budget in 1_000..1_200 {
        let messages = trim(original.clone(), budget);
        let parsed = parse(&messages);
        let partial = parsed["partial"]
            .as_str()
            .unwrap_or_else(|| panic!("budget {budget}: no partial string"));
        assert!(
            original.starts_with(partial),
            "budget {budget}: the partial is not a prefix of the original"
        );
        if let Some(&next) = original.as_bytes().get(partial.len()) {
            assert_ne!(
                next & 0b1100_0000,
                0b1000_0000,
                "budget {budget}: the cut split a multi-byte character"
            );
        }
    }
}

/// A result trimmed more than once — the preflight re-trims a message it
/// already trimmed — refines the same envelope instead of nesting a second one
/// inside the first. Nesting would re-escape the surviving text at every pass
/// and grow the provider payload without bound; the original size and the
/// partial both stay the *original* result's.
#[test]
fn a_result_trimmed_again_refines_the_envelope_rather_than_nesting_it() {
    let original = json_result(&"x".repeat(32 * 1024));
    // A budget so tight the trim's own loop passes over the message repeatedly.
    let messages = trim(original.clone(), 1_200);
    let parsed = parse(&messages);
    assert_eq!(parsed["original_bytes"], serde_json::json!(original.len()));
    let partial = parsed["partial"].as_str().expect("a partial string");
    assert!(
        original.starts_with(partial),
        "the partial must be a prefix of the original result, not of an envelope"
    );
    assert!(
        !partial.contains("truncated"),
        "no envelope nested in the partial"
    );
}

/// The envelope's size bound is priced with `escaped_char_len`, so that must
/// agree with the serializer byte for byte — for every character, not just the
/// ones the samples happen to use. A mismatch would let an envelope exceed the
/// room the trim computed for it.
#[test]
fn escaped_char_len_matches_the_serializer() {
    let chars = (0u32..0x300)
        .chain([0x2028, 0x2029, 0xfeff, 0x1_0000, 0x10_ffff])
        .filter_map(char::from_u32);
    for ch in chars {
        let quoted = serde_json::to_string(&ch.to_string()).expect("a char serializes");
        assert_eq!(
            super::super::trim_envelope::escaped_char_len(ch),
            quoted.len() - 2,
            "escaping cost of {ch:?} (U+{:04X})",
            ch as u32
        );
    }
}

/// Non-JSON content keeps the same form: the longest prefix plus the marker.
#[test]
fn a_truncated_non_json_tool_result_keeps_the_marker() {
    let original = "X".repeat(8 * 1024);
    let messages = trim(original.clone(), 1_500);
    let body = &reply(&messages).content;
    let head = body
        .strip_suffix(TRUNCATED_MARKER)
        .expect("non-JSON content keeps the marker form");
    assert!(original.starts_with(head));
    assert!(total(&messages) <= 1_500);
}
