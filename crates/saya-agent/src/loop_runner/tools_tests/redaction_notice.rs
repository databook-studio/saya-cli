// -- the redaction announcement (D8 follow-up): the model must be able to --
// -- tell a masked value from corruption, and must never write the --------
// -- placeholder back over the real one. -----------------------------------

/// A tool result whose content is secret-shaped reaches the model with the
/// value masked *and* a fixed note naming how many replacements were made,
/// so the model can tell masking from corruption instead of "fixing" a
/// `[redacted]` it reads back. Two secrets in the same result count 2, not
/// one note per secret.
#[test]
fn a_redacted_tool_result_tells_the_model() {
    let one = serde_json::json!({"value": "token=abc123"});
    let (message, _) = tool_message("c1".into(), one, usize::MAX);
    assert!(
        message.content.contains("[redacted]"),
        "the secret must still be masked: {}",
        message.content
    );
    assert!(
        message
            .content
            .contains("[saya: 1 secret-shaped value(s) in this result were replaced with [redacted]; the source is unchanged — do not write [redacted] back]"),
        "the note must name the exact count: {}",
        message.content
    );

    let two = serde_json::json!({"value": "token=abc123 password=xyz"});
    let (message, _) = tool_message("c2".into(), two, usize::MAX);
    assert!(
        message
            .content
            .contains("[saya: 2 secret-shaped value(s) in this result were replaced with [redacted]; the source is unchanged — do not write [redacted] back]"),
        "two secrets must count 2: {}",
        message.content
    );
}

/// Nothing secret-shaped in the result: the content is byte-identical to the
/// un-noted redaction — no note is ever appended for a clean result.
#[test]
fn an_unredacted_tool_result_is_unchanged() {
    let result = serde_json::json!({"value": "SELECT 1 WHERE status='done'"});
    let (message, _) = tool_message("c1".into(), result.clone(), usize::MAX);
    let expected = serde_json::to_string(&result).unwrap();
    assert_eq!(
        message.content, expected,
        "a clean result must be byte-identical, with no note prepended: {}",
        message.content
    );
    assert!(
        !message.content.contains("[saya:"),
        "no note may appear when nothing was redacted: {}",
        message.content
    );
}
