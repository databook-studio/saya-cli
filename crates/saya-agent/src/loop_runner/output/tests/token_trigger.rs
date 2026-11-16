//! Compaction is keyed to a fraction of the known context window: the
//! conversation compacts when its token estimate crosses the trigger, even
//! when it is far below the byte budget. The byte budget stays a hard ceiling,
//! and the newest tool-result group survives every compaction.

use super::super::*;
use super::compaction::group;

const PROTECTED: usize = 2;
const KIB: usize = 1024;

/// Bytes the conversation may take at `tokens`, using the same estimate the
/// trigger uses.
fn bytes_for_tokens(tokens: u64) -> usize {
    (tokens as f64 * crate::providers::framing::ESTIMATE_BYTES_PER_TOKEN).ceil() as usize
}

fn conversation() -> Vec<crate::ChatMessage> {
    vec![
        crate::ChatMessage::text("system", "sys"),
        crate::ChatMessage::text("user", "ask"),
    ]
}

fn result(bytes: usize) -> serde_json::Value {
    serde_json::json!({"row_count":1,"columns":["one"],"rows":[["x".repeat(bytes)]]})
}

fn add_groups(messages: &mut Vec<crate::ChatMessage>, count: usize, result_bytes: usize) {
    for index in 0..count {
        messages.extend(group(
            &format!("call-{index:03}"),
            "SELECT 1",
            result(result_bytes),
        ));
    }
}

fn size(messages: &[crate::ChatMessage]) -> usize {
    messages.iter().map(message_size).sum()
}

fn digests(messages: &[crate::ChatMessage]) -> usize {
    messages
        .iter()
        .filter(|message| crate::loop_runner::compaction::is_digest(message))
        .count()
}

/// The last complete tool-result group, as the part of the conversation the
/// next request needs whole.
fn newest_group(messages: &[crate::ChatMessage]) -> Vec<crate::ChatMessage> {
    let start = messages
        .iter()
        .rposition(|message| !message.tool_calls.is_empty())
        .expect("a newest tool-call turn exists");
    messages[start..].to_vec()
}

/// A window large enough that its 50% trigger sits well below the byte
/// budget's own 90% trigger. A conversation between the two marks must
/// compact: the byte budget alone would leave it untouched.
#[test]
fn a_large_window_compacts_at_the_token_fraction_before_the_byte_budget() {
    let window_tokens = 1_000_000u64;
    let trigger_tokens = window_tokens / 2;
    let trigger_bytes = bytes_for_tokens(trigger_tokens);
    let byte_budget = 4 * 1024 * 1024;
    assert!(
        trigger_bytes < byte_budget / 100 * 90,
        "setup: the token trigger ({trigger_bytes}) must bind before the byte trigger"
    );

    let mut messages = conversation();
    add_groups(&mut messages, 27, 64 * KIB);
    let total = size(&messages);
    assert!(
        total > trigger_bytes && total < byte_budget / 100 * 90,
        "setup: the conversation must sit between the token and byte triggers: {total}"
    );
    let newest = newest_group(&messages);

    trim_to_budget(
        &mut messages,
        ContextBudget::new(byte_budget, Some(trigger_tokens)),
        PROTECTED,
        None,
        &Default::default(),
    );

    assert_eq!(
        digests(&messages),
        1,
        "the token trigger must run one compaction"
    );
    assert_eq!(
        &messages[..PROTECTED],
        conversation().as_slice(),
        "the protected prefix stays byte-identical"
    );
    assert_eq!(
        &messages[messages.len() - newest.len()..],
        newest.as_slice(),
        "the newest group the next step needs stays byte-identical"
    );
    let drain = trigger_bytes * 60 / 90;
    assert!(
        size(&messages) <= drain,
        "a token-triggered compaction drains below its trigger: {} vs {drain}",
        size(&messages)
    );
}

/// The same window and byte budget, but the conversation stays below the
/// token trigger: it must be left untouched.
#[test]
fn a_large_window_below_the_token_fraction_is_untouched() {
    let window_tokens = 1_000_000u64;
    let trigger_tokens = window_tokens / 2;
    let trigger_bytes = bytes_for_tokens(trigger_tokens);
    let byte_budget = 4 * 1024 * 1024;

    let mut messages = conversation();
    add_groups(&mut messages, 26, 64 * KIB);
    let total = size(&messages);
    assert!(
        total < trigger_bytes,
        "setup: the conversation must stay below the token trigger: {total}"
    );
    let snapshot = messages.clone();

    trim_to_budget(
        &mut messages,
        ContextBudget::new(byte_budget, Some(trigger_tokens)),
        PROTECTED,
        None,
        &Default::default(),
    );

    assert_eq!(
        messages, snapshot,
        "a conversation below the token trigger is untouched"
    );
}

/// A token trigger far above the byte budget cannot raise the byte ceiling:
/// the conversation is still compacted and truncated to fit.
#[test]
fn the_byte_budget_stays_a_hard_ceiling_with_a_large_window() {
    let byte_budget = 64 * KIB;
    let trigger_tokens = 500_000_000u64;

    let mut messages = conversation();
    add_groups(&mut messages, 12, 8 * KIB);
    let total = size(&messages);
    assert!(
        total > byte_budget,
        "setup: the conversation must exceed the hard byte ceiling: {total}"
    );
    let newest = newest_group(&messages);

    trim_to_budget(
        &mut messages,
        ContextBudget::new(byte_budget, Some(trigger_tokens)),
        PROTECTED,
        None,
        &Default::default(),
    );

    assert_eq!(digests(&messages), 1, "the byte ceiling still compacts");
    assert_eq!(
        &messages[messages.len() - newest.len()..],
        newest.as_slice(),
        "the newest group stays byte-identical"
    );
    assert!(
        size(&messages) <= byte_budget,
        "the hard byte ceiling binds: {} vs {byte_budget}",
        size(&messages)
    );
}

/// A token-triggered compaction keeps every retained call paired with its
/// result — the model must never see a call whose answer was dropped.
#[test]
fn a_token_triggered_compaction_keeps_the_latest_tool_results() {
    let window_tokens = 10_000u64;
    let trigger_tokens = window_tokens / 2;
    let trigger_bytes = bytes_for_tokens(trigger_tokens);
    let byte_budget = 4 * 1024 * 1024;

    let mut messages = conversation();
    add_groups(&mut messages, 4, 8 * KIB);
    assert!(
        size(&messages) > trigger_bytes,
        "setup: the conversation must cross the token trigger"
    );
    let newest = newest_group(&messages);

    trim_to_budget(
        &mut messages,
        ContextBudget::new(byte_budget, Some(trigger_tokens)),
        PROTECTED,
        None,
        &Default::default(),
    );

    assert_eq!(
        digests(&messages),
        1,
        "the token trigger must run one compaction"
    );
    assert_eq!(
        &messages[messages.len() - newest.len()..],
        newest.as_slice()
    );
    for call in messages.iter().flat_map(|message| &message.tool_calls) {
        assert!(
            messages.iter().any(|result| {
                result.role == "tool" && result.tool_call_id.as_deref() == Some(&call.id)
            }),
            "tool call {} was separated from its result",
            call.id
        );
    }
}

/// Replayed reasoning travels with the assistant turn on the wire, so the
/// context estimate must count it. This trace alone pushes the conversation
/// over the byte mark: without it the same conversation is untouched.
fn add_replayed_reasoning(messages: &mut [crate::ChatMessage], bytes: usize) {
    let oldest = messages
        .iter_mut()
        .find(|message| message.role == "assistant")
        .expect("a tool group has an assistant turn");
    let trace = "r".repeat(bytes);
    *oldest = oldest.clone().with_reasoning(Some(trace));
}

/// The estimate with the replayed traces removed, so a test can state that
/// the content alone sits below a mark.
fn content_size(messages: &[crate::ChatMessage]) -> usize {
    let mut without = messages.to_vec();
    for message in &mut without {
        message.reasoning = None;
        message.reasoning_signature = None;
    }
    size(&without)
}

#[test]
fn replayed_reasoning_counts_toward_the_byte_trigger() {
    let byte_budget = 39 * KIB;
    let trigger = byte_budget / 100 * 90;

    let mut messages = conversation();
    add_groups(&mut messages, 2, 2 * KIB);
    add_replayed_reasoning(&mut messages, 32 * KIB);

    assert!(
        content_size(&messages) < trigger,
        "setup: the content alone must sit below the byte mark: {} vs {trigger}",
        content_size(&messages)
    );
    assert!(
        size(&messages) > trigger,
        "the replayed trace is part of the request and must count: {} vs {trigger}",
        size(&messages)
    );

    trim_to_budget(
        &mut messages,
        ContextBudget::new(byte_budget, None),
        PROTECTED,
        None,
        &Default::default(),
    );

    assert_eq!(
        digests(&messages),
        1,
        "the trace-bearing group is evicted by the byte mark"
    );
    assert!(
        size(&messages) <= byte_budget / 100 * 60,
        "the compaction drains to the byte drain: {}",
        size(&messages)
    );
}

#[test]
fn replayed_reasoning_counts_toward_the_token_trigger() {
    let window_tokens = 20_000u64;
    let trigger_tokens = window_tokens / 2;
    let trigger = bytes_for_tokens(trigger_tokens);
    let byte_budget = 4 * 1024 * 1024;

    let mut messages = conversation();
    add_groups(&mut messages, 2, 2 * KIB);
    add_replayed_reasoning(&mut messages, 32 * KIB);

    assert!(
        content_size(&messages) < trigger,
        "setup: the content alone must sit below the token mark: {} vs {trigger}",
        content_size(&messages)
    );
    assert!(
        size(&messages) > trigger,
        "the replayed trace is part of the request and must count: {} vs {trigger}",
        size(&messages)
    );

    trim_to_budget(
        &mut messages,
        ContextBudget::new(byte_budget, Some(trigger_tokens)),
        PROTECTED,
        None,
        &Default::default(),
    );

    assert_eq!(
        digests(&messages),
        1,
        "the trace-bearing group is evicted by the token mark"
    );
}
