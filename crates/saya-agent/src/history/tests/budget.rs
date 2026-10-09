use super::super::*;
use super::BUDGET;

/// A context block counts toward the conversation budget the same way the
/// prompt does: a block larger than the budget leaves no room for history, but
/// the run no longer fails closed — the loop bounds growth during execution.
#[test]
fn context_blocks_count_toward_the_byte_budget_without_failing_closed() {
    let big = ContextBlock {
        label: "database-contracts".into(),
        body: "y".repeat(BUDGET),
        truncated: false,
    };
    let result = build_messages(None, &[big], "prompt", &[], BUDGET);
    assert!(
        result.is_ok(),
        "oversized context must not fail closed: {result:?}"
    );
    let messages = result.unwrap();
    assert_eq!(
        messages.len(),
        2,
        "no history is included once the block fills the budget"
    );
}

// --- a context block larger than the former ceiling must not break a turn ----

/// A block larger than the former 32 KiB ceiling plus an ordinary prompt
/// still builds: with the start-of-run cap gone, the loop bounds the
/// conversation during execution rather than refusing the turn up front.
#[test]
fn a_large_context_block_with_an_ordinary_prompt_builds() {
    let block = ContextBlock {
        label: "database-contracts".into(),
        body: "y".repeat(60 * 1024),
        truncated: true,
    };
    let result = build_messages(None, &[block], "show me orders by month", &[], BUDGET);
    assert!(
        result.is_ok(),
        "a large context block must not break an ordinary prompt: {result:?}"
    );
}

/// `turn_bytes` is the exact size `build_messages` assembles for the system
/// and user turns, so a caller can measure what a block will cost before
/// building. The two share one accounting and cannot drift: the size
/// `turn_bytes` reports is the size that lands in the built messages.
#[test]
fn turn_bytes_matches_what_build_messages_assembles() {
    let body = "x".repeat(2048);
    let block = ContextBlock {
        label: "database-contracts".into(),
        body: body.clone(),
        truncated: false,
    };
    let measured = turn_bytes(None, std::slice::from_ref(&block), "prompt");
    let messages = build_messages(None, &[block], "prompt", &[], BUDGET).unwrap();
    let built: usize = messages.iter().map(message_bytes).sum();
    assert_eq!(
        measured, built,
        "turn_bytes must report the exact size build_messages assembles"
    );
}
