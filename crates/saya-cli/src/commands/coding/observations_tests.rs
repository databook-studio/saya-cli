//! The redacted, bounded `reason_text` one durable tool record carries: the
//! tool-result error text the model received, scrubbed by the store's own
//! redactor and cut to 1,024 bytes on a UTF-8 boundary with a trailing marker.
use super::bounded_reason_text;
use saya_agent::{MAX_TOOL_REASON_BYTES, TOOL_REASON_TRUNCATION_MARKER};

#[test]
fn a_five_kibibyte_error_text_is_cut_to_the_bound_with_a_trailing_marker() {
    let text = "x".repeat(5 * 1024);
    let reason = bounded_reason_text(&text);
    assert!(
        reason.len() <= MAX_TOOL_REASON_BYTES,
        "a 5 KiB reason is cut to the bound: {} bytes",
        reason.len()
    );
    assert!(
        reason.ends_with(TOOL_REASON_TRUNCATION_MARKER),
        "a cut reason says so: {:?}",
        &reason[reason.len().saturating_sub(32)..]
    );
    assert_eq!(
        reason.len(),
        MAX_TOOL_REASON_BYTES,
        "the cut uses the whole bound"
    );
}

#[test]
fn a_cut_lands_on_a_utf8_boundary_and_never_splits_a_character() {
    // Every character is two bytes, so the byte bound falls inside one.
    let text = "é".repeat(2 * 1024);
    let reason = bounded_reason_text(&text);
    assert!(reason.len() <= MAX_TOOL_REASON_BYTES);
    assert!(
        reason.ends_with(TOOL_REASON_TRUNCATION_MARKER),
        "the text is longer than the bound, so it is cut"
    );
    assert!(
        (reason.len() - TOOL_REASON_TRUNCATION_MARKER.len()).is_multiple_of(2),
        "the kept prefix is whole characters, never a split one"
    );
    // A text inside the bound is kept whole and carries no marker.
    let short = bounded_reason_text("the child authority does not grant this tool");
    assert_eq!(short, "the child authority does not grant this tool");
}

#[test]
fn a_secret_shaped_value_in_the_text_is_redacted() {
    let reason = bounded_reason_text(
        "the shell tool failed: password=hunter2 and Authorization: Bearer abc123",
    );
    assert!(
        !reason.contains("hunter2"),
        "the store's redactor scrubs a keyed value: {reason:?}"
    );
    assert!(
        !reason.contains("abc123"),
        "the store's redactor scrubs a credential header: {reason:?}"
    );
    assert!(
        reason.contains("[redacted]"),
        "the scrubbed text names the replacement: {reason:?}"
    );
    // Redaction runs before the bound. Replacing a one-byte value with the
    // eleven-byte marker lengthens the text, so the bound is re-applied after.
    let lengthened = "password=a ".repeat(4 * 1024);
    assert!(lengthened.len() > MAX_TOOL_REASON_BYTES);
    let reason = bounded_reason_text(&lengthened);
    assert!(
        reason.len() <= MAX_TOOL_REASON_BYTES,
        "redaction can lengthen the text, so the bound is re-applied: {} bytes",
        reason.len()
    );
}
