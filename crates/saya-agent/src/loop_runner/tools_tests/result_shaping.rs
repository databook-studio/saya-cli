// -- the shaping extraction (R3): the capture hook must shape the model's --
// -- view with the exact function the loop uses, so both see the same ------
// -- bytes. The pin: the extracted function reproduces the pre-extraction --
// -- `tool_message` body byte for byte. ------------------------------------

/// The pre-extraction `tool_message` shaping, embedded as the reference:
/// serialize, cut to the cap on a char boundary with the visible marker, then
/// redact what remains and prepend the note when anything was replaced. Only
/// `redact_counted` and `tool_message_cap` — unchanged external contracts,
/// pinned by their own tests — are called from production; everything else
/// here is the literal old code, so the comparison cannot drift with it.
fn old_tool_message_shaping(result: &Value, byte_budget: usize) -> (String, bool) {
    let cap = tool_message_cap(byte_budget);
    let text = serde_json::to_string(result)
        .unwrap_or_else(|_| "{\"error\":\"tool result unavailable\"}".into());
    let (content, truncated) = if text.len() <= cap {
        (text, false)
    } else {
        let marker = "…[truncated: tool result exceeded the conversation byte budget]";
        let head = cap.saturating_sub(marker.len());
        let mut idx = head;
        if idx >= text.len() {
            idx = text.len();
        }
        while idx > 0 && !text.is_char_boundary(idx) {
            idx -= 1;
        }
        let mut truncated = String::from(&text[..idx]);
        truncated.push_str(marker);
        (truncated, true)
    };
    let (content, redacted_count) = redact_counted(&content);
    let content = if redacted_count > 0 {
        format!(
            "[saya: {redacted_count} secret-shaped value(s) in this result were replaced with \
             [redacted]; the source is unchanged — do not write [redacted] back]\n\n{content}"
        )
    } else {
        content
    };
    (content, truncated)
}

/// The extraction pin (R3): `shape_tool_result` returns exactly the bytes the
/// loop's `tool_message` has always put in the model's context — across a
/// clean result, a redacted one (note prepended, count named), whole-at-the-
/// cap, one-byte-over, a tight budget cutting on a multi-byte boundary, and a
/// secret that survives into the truncated head (redaction after the cut).
/// `tool_message` must shape through the same function — no second
/// implementation to drift.
#[test]
fn shaping_matches_tool_message_and_caps_cut_results() {
    let cap = tool_message_cap(usize::MAX);
    let cases: Vec<(Value, usize, usize)> = vec![
        // A clean, small result — the common case: whole, no redactions.
        (
            serde_json::json!({"columns": ["id"], "rows": [[1], [2]], "row_count": 2}),
            usize::MAX,
            0,
        ),
        // Two secret-shaped values: note prepended, count 2.
        (
            serde_json::json!({"value": "token=abc123 password=xyz"}),
            usize::MAX,
            2,
        ),
        // Exactly at the cap: whole.
        (Value::String("x".repeat(cap - 2)), usize::MAX, 0),
        // One byte over the cap: cut with the loop's own marker.
        (Value::String("x".repeat(cap - 1)), usize::MAX, 0),
        // A tight budget cuts inside a multi-byte sequence: the boundary is
        // floored, never split.
        (serde_json::json!({"text": "é".repeat(64)}), 64, 0),
        // A tight budget whose truncated head still carries a secret: the
        // redaction runs on the CUT text and the note prepends — order pinned.
        (
            serde_json::json!({"text": format!("token=abc123 {}", "y".repeat(200))}),
            120,
            1,
        ),
        // Trivial shapes stay trivial.
        (serde_json::json!({}), usize::MAX, 0),
        (Value::Null, usize::MAX, 0),
    ];
    for (value, budget, redactions) in cases {
        let shaped = shape_tool_result(&value, budget);
        if !shaped.truncated {
            let (old_content, old_truncated) = old_tool_message_shaping(&value, budget);
            assert_eq!(shaped.text.as_bytes(), old_content.as_bytes());
            assert_eq!(shaped.truncated, old_truncated);
        }
        assert!(
            shaped.text.len() <= tool_message_cap(budget),
            "a cut result must remain under the cap (budget {budget})"
        );
        if !shaped.truncated {
            assert_eq!(
                shaped.redactions, redactions,
                "the redaction count must match the old body (budget {budget})"
            );
        }
        // `tool_message` has no second implementation: its content is exactly
        // the extracted shaping's text.
        let (message, truncated) = tool_message("c1".into(), value.clone(), budget);
        assert_eq!(
            message.content, shaped.text,
            "tool_message must shape through the one function (budget {budget})"
        );
        assert_eq!(truncated, shaped.truncated);
    }
}

/// The final model-facing text, not merely its serialized prefix, must fit its
/// cap. A marker, redaction replacement, or redaction notice cannot turn a
/// bounded result into an oversized provider message.
#[test]
fn shaping_never_exceeds_its_cap_for_tiny_unicode_and_redacted_results() {
    for (value, budget) in [
        (serde_json::json!({"text": "é".repeat(64)}), 1),
        (serde_json::json!({"token": "token=abc123"}), 8),
        (serde_json::json!({"text": "x".repeat(256)}), 16),
    ] {
        let shaped = shape_tool_result(&value, budget);
        assert!(
            shaped.text.len() <= tool_message_cap(budget),
            "final shaped text exceeds {} bytes: {}",
            tool_message_cap(budget),
            shaped.text.len()
        );
        assert!(
            std::str::from_utf8(shaped.text.as_bytes()).is_ok(),
            "shaped output must remain UTF-8"
        );
    }
}

/// A serializer may offer an entire JSON string in one write. The bounded
/// writer still preserves the useful prefix of that one write, rather than
/// keeping only structural JSON fragments before the truncation marker.
#[test]
fn a_single_large_unicode_write_keeps_a_visible_prefix() {
    let shaped = shape_tool_result(&Value::String("星".repeat(256)), 64);
    assert!(shaped.truncated, "the oversized value must be marked cut");
    assert!(
        shaped.text.contains('星'),
        "a useful Unicode prefix survives"
    );
    assert!(shaped.text.contains("[truncated]"), "the cut stays visible");
    assert!(
        shaped.text.len() <= 64,
        "the final text stays within the cap"
    );
}

/// A tool-turn budget is divided before execution, so every accepted call has
/// a deterministic share and their retained model-facing contents cannot add
/// up past the turn's aggregate allowance.
#[test]
fn turn_result_caps_share_the_aggregate_budget_without_a_minimum() {
    assert_eq!(turn_result_caps(10, 3), vec![4, 3, 3]);
    assert_eq!(turn_result_caps(2, 3), vec![1, 1, 0]);
    let caps = turn_result_caps(usize::MAX, 3);
    assert_eq!(caps, vec![MAX_TOOL_MESSAGE_BYTES; 3]);
    assert!(caps.iter().sum::<usize>() <= 256 * 1024);
}
