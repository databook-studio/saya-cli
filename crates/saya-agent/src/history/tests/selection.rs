use super::super::*;
use super::BUDGET;

/// With the count cap (`MAX_HISTORY_MESSAGES = 20`) removed, a long run of
/// small turns replays in full when its bytes fit: history is bounded by
/// bytes, not by message count. Twenty-four pairs (48 messages) would have
/// been truncated to 10 pairs under the old cap.
#[test]
fn keeps_newest_complete_turns_with_stable_bounds() {
    let history = (0..24)
        .flat_map(|index| {
            [
                ChatMessage::text("user", format!("u{index}")),
                ChatMessage::text("assistant", format!("a{index}")),
            ]
        })
        .collect::<Vec<_>>();
    let messages = build_messages(None, &[], "current", &history, BUDGET).unwrap();
    assert_eq!(
        messages.len(),
        2 + history.len(),
        "every pair must replay when its bytes fit the budget"
    );
    assert_eq!(messages[1].content, "u0");
    assert_eq!(messages[messages.len() - 2].content, "a23");
    assert_eq!(messages[1].role, "user");
    assert_eq!(messages[2].role, "assistant");
    assert_eq!(messages[3].role, "user");
    assert!(messages.iter().map(message_bytes).sum::<usize>() <= BUDGET);
}

/// Byte-boundary pin: when history bytes exceed the budget, the oldest
/// pairs are dropped first and exactly the newest pairs that fit are kept.
#[test]
fn byte_budget_truncates_oldest_first_at_byte_boundary() {
    let body = "x".repeat(1000);
    let history = (0..5)
        .flat_map(|index| {
            [
                ChatMessage::text("user", format!("u{index}-{body}")),
                ChatMessage::text("assistant", format!("a{index}-{body}")),
            ]
        })
        .collect::<Vec<_>>();
    let pair_bytes = history[history.len() - 2..]
        .iter()
        .map(message_bytes)
        .sum::<usize>();
    // Room for two pairs plus half a pair: the two newest pairs fit, the
    // third does not.
    let budget = turn_bytes(None, &[], "current") + 2 * pair_bytes + pair_bytes / 2;
    let messages = build_messages(None, &[], "current", &history, budget).unwrap();
    assert_eq!(
        messages.len(),
        2 + 4,
        "only the two newest pairs fit the byte budget"
    );
    assert!(messages[1].content.starts_with("u3-"));
    assert!(messages[4].content.starts_with("a4-"));
    assert!(messages.iter().map(message_bytes).sum::<usize>() <= budget);
}

/// `as_chunks::<2>()` pin: a user/assistant pair is never split. When the
/// byte boundary falls mid-pair, the whole older pair is dropped rather
/// than keeping a lone user message.
#[test]
fn history_pair_is_never_split_at_the_byte_boundary() {
    let body = "x".repeat(1000);
    let history = (0..3)
        .flat_map(|index| {
            [
                ChatMessage::text("user", format!("u{index}-{body}")),
                ChatMessage::text("assistant", format!("a{index}-{body}")),
            ]
        })
        .collect::<Vec<_>>();
    let pair_bytes = history[history.len() - 2..]
        .iter()
        .map(message_bytes)
        .sum::<usize>();
    // Room for one pair plus nearly another pair's worth: a second message
    // would fit, but never half a pair.
    let budget = turn_bytes(None, &[], "current") + 2 * pair_bytes - 1;
    let messages = build_messages(None, &[], "current", &history, budget).unwrap();
    assert_eq!(
        messages.len(),
        4,
        "only the newest complete pair fits; it must not be split"
    );
    assert_eq!(messages[1].role, "user");
    assert!(messages[1].content.starts_with("u2-"));
    assert_eq!(messages[2].role, "assistant");
    assert!(messages[2].content.starts_with("a2-"));
}

#[test]
fn rejects_tool_history_and_oversized_current_prompt() {
    let history = vec![ChatMessage {
        role: "tool".into(),
        content: "row-sentinel".into(),
        tool_calls: Vec::new(),
        tool_call_id: Some("call".into()),
    }];
    assert!(matches!(
        build_messages(None, &[], "ok", &history, BUDGET),
        Err(AgentError::InvalidHistory)
    ));
}

/// The start-of-run cap on system + user is gone: the loop bounds the
/// conversation with `context_byte_budget`, so a prompt larger than the former
/// 32 KiB ceiling builds (with no history) instead of failing closed.
#[test]
fn an_oversized_prompt_builds_without_failing_closed() {
    let huge = "x".repeat(64 * 1024);
    let result = build_messages(None, &[], &huge, &[], 64 * 1024);
    assert!(
        result.is_ok(),
        "an oversized prompt must not fail closed: {result:?}"
    );
    let messages = result.unwrap();
    assert_eq!(
        messages.len(),
        2,
        "no history is included when the prompt alone fills the budget"
    );
}

#[test]
fn cumulative_byte_boundary_keeps_only_newest_contiguous_suffix() {
    let large = "x".repeat(BUDGET / 2);
    let history = vec![
        ChatMessage::text("user", "old"),
        ChatMessage::text("assistant", "old-answer"),
        ChatMessage::text("user", large.clone()),
        ChatMessage::text("assistant", large),
    ];
    let messages = build_messages(None, &[], "current", &history, BUDGET).unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1].content, "current");
}
