use super::{
    MAX_TOOL_CALL_ID_BYTES, MAX_TOOL_TRANSCRIPT_BYTES, MAX_TOOL_TRANSCRIPT_FIELD_BYTES,
    TOOL_TRANSCRIPT_ELISION_MARKER, ToolCallTranscript, bound_tool_transcript,
};

#[test]
fn a_text_inside_the_bound_is_kept_whole_and_carries_no_marker() {
    let text = "{\"path\":\"a.txt\",\"content\":\"ok\"}";
    assert_eq!(bound_tool_transcript(text, 1024), text);
    assert_eq!(bound_tool_transcript(text, text.len()), text);
}

#[test]
fn a_text_over_the_bound_keeps_both_ends_and_names_what_it_dropped() {
    let text = format!("HEAD{}TAIL", "x".repeat(8 * 1024));
    let bounded = bound_tool_transcript(&text, MAX_TOOL_TRANSCRIPT_FIELD_BYTES);
    assert!(
        bounded.len() <= MAX_TOOL_TRANSCRIPT_FIELD_BYTES,
        "the bound holds: {} bytes",
        bounded.len()
    );
    assert!(bounded.starts_with("HEAD"), "{bounded:?}");
    assert!(bounded.ends_with("TAIL"), "{bounded:?}");
    assert!(
        bounded.contains(TOOL_TRANSCRIPT_ELISION_MARKER),
        "{bounded:?}"
    );
}

#[test]
fn a_cut_lands_on_utf8_boundaries_and_never_splits_a_character() {
    // Every character is two bytes, so both cut points fall inside one.
    let text = "é".repeat(4 * 1024);
    let bounded = bound_tool_transcript(&text, MAX_TOOL_TRANSCRIPT_FIELD_BYTES);
    assert!(bounded.len() <= MAX_TOOL_TRANSCRIPT_FIELD_BYTES);
    let (head, rest) = bounded.split_once(TOOL_TRANSCRIPT_ELISION_MARKER).unwrap();
    assert!(head.len().is_multiple_of(2), "the head is whole characters");
    assert!(rest.len().is_multiple_of(2), "the tail is whole characters");
}

#[test]
fn a_cap_too_small_for_the_marker_yields_nothing_rather_than_a_partial_marker() {
    let text = "0123456789".repeat(4);
    assert_eq!(bound_tool_transcript(&text, 4), "");
    assert_eq!(
        bound_tool_transcript(&text, TOOL_TRANSCRIPT_ELISION_MARKER.len() + 1),
        ""
    );
}

#[test]
fn a_transcript_bounds_the_call_id_the_arguments_and_the_result() {
    let transcript = ToolCallTranscript::new(
        &"c".repeat(4 * 1024),
        &"a".repeat(4 * 1024),
        Some(&"r".repeat(4 * 1024)),
        17,
    );
    assert_eq!(transcript.call_id().len(), MAX_TOOL_CALL_ID_BYTES);
    assert_eq!(
        transcript.arguments().len(),
        MAX_TOOL_TRANSCRIPT_FIELD_BYTES
    );
    assert_eq!(
        transcript.result().map(str::len),
        Some(MAX_TOOL_TRANSCRIPT_FIELD_BYTES)
    );
    assert_eq!(transcript.duration_ms(), 17);
    assert!(
        transcript
            .call_id()
            .contains(TOOL_TRANSCRIPT_ELISION_MARKER)
    );
}

#[test]
fn a_transcript_without_a_result_omits_the_key_entirely() {
    let transcript = ToolCallTranscript::new("call-1", "{}", None, 0);
    assert_eq!(transcript.result(), None);
    let serialized = serde_json::to_string(&transcript).unwrap();
    assert!(!serialized.contains("result"), "{serialized}");
    assert!(serialized.contains("call-1") && serialized.contains("duration_ms"));
}

#[test]
fn a_transcript_with_maximal_fields_still_serializes_inside_the_whole_bound() {
    // Two maximal fields of quotes escape to twice their bound each, so the
    // field budget alone cannot hold the payload: the fit loop cuts again.
    let quotes = "\"".repeat(MAX_TOOL_TRANSCRIPT_FIELD_BYTES);
    let transcript = ToolCallTranscript::new("call-1", &quotes, Some(&quotes), 3);
    let serialized = serde_json::to_vec(&transcript).unwrap();
    assert!(
        serialized.len() <= MAX_TOOL_TRANSCRIPT_BYTES,
        "the payload fits the whole-transcript bound: {} bytes",
        serialized.len()
    );
    assert!(
        transcript
            .arguments()
            .contains(TOOL_TRANSCRIPT_ELISION_MARKER),
        "a cut field says what it dropped: {:?}",
        transcript.arguments()
    );
    assert!(transcript.arguments().len() <= MAX_TOOL_TRANSCRIPT_FIELD_BYTES);
    assert!(transcript.call_id() == "call-1" && transcript.duration_ms() == 3);
}
