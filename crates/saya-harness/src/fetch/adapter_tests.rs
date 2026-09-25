//! The fetch lane's bound and backstop, at the envelope level.
//!
//! The sentinels are the scheme's fixed, public constants; the harness's
//! tests hardcode the literals the way `fetch_tool.rs` does.

use super::super::limits::ENVELOPE_SLACK;
use super::*;
use saya_agent::MAX_TOOL_MESSAGE_BYTES;

const CONTEXT_CLOSE: &str = "<<<CONTEXT_BLOCK_END>>>";

/// The drift pin, from the harness side: the lane's cap is exactly the
/// loop's exported tool-message cap — the same number the loop truncates
/// at — and the wired body bound sits below it minus the envelope slack,
/// so a fetch success at the bound always fits the lane.
#[test]
fn the_lane_bound_is_below_the_loop_s_tool_message_cap_by_construction() {
    let cap = FetchLimits::tool_lane_cap();
    assert_eq!(
        cap, MAX_TOOL_MESSAGE_BYTES,
        "the lane cap is the loop's exported constant: the harness bound and \
         the loop's truncation point cannot drift"
    );
    let bound = FetchLimits::for_tool_lane();
    assert_eq!(
        bound.max_total_bytes,
        cap - ENVELOPE_SLACK,
        "the wired body bound is the cap minus the envelope slack"
    );
    assert!(bound.max_total_bytes < cap);
}

/// A fetch that fits arrives whole: the envelope under the cap, the block
/// closed, `truncated` false — the pre-bound's normal path, so the loop's
/// own truncation marker never fires on a fetch result.
#[test]
fn a_result_under_the_cap_arrives_whole_and_closed() {
    let mut block = ContextBlock {
        label: "http-fetch example.com".into(),
        body: "honest body".into(),
        truncated: false,
    };
    let cap = FetchLimits::tool_lane_cap();
    let value = bounded_fetch_envelope("https://example.com/doc", &mut block, cap)
        .expect("an ordinary fetch fits the lane");
    assert_eq!(
        value["content"]
            .as_str()
            .unwrap()
            .matches(CONTEXT_CLOSE)
            .count(),
        1,
        "the block arrives closed: {value}"
    );
    assert!(!block.truncated, "the whole body arrived; nothing was cut");
    assert!(
        value.to_string().len() <= cap,
        "the envelope fits the tool-message cap: {} vs {cap}",
        value.to_string().len()
    );
}

/// The backstop: a body whose escaping doubles its rendered size blows past
/// the cap even though the raw body was at the wired bound. The cut lands
/// on the *body*, the block is flagged so its own in-block marker renders,
/// and the emitted envelope fits the cap with the block closed — the loop's
/// out-of-structure truncation marker must never appear.
#[test]
fn the_backstop_cuts_the_body_and_the_block_stays_closed() {
    // Backslash-heavy: escape_block_text doubles every backslash, so the
    // rendered block is twice the raw body — over the cap despite the body
    // being within the wired bound.
    let mut block = ContextBlock {
        label: "http-fetch hostile.example.com".into(),
        body: "\\".repeat(FetchLimits::for_tool_lane().max_total_bytes),
        truncated: false,
    };
    let cap = FetchLimits::tool_lane_cap();
    let value = bounded_fetch_envelope("https://hostile.example.com/page", &mut block, cap)
        .expect("the backstop must deliver a fitting result");
    let serialized = value.to_string();
    assert!(
        serialized.len() <= cap,
        "the emitted envelope fits the cap: {} vs {cap}",
        serialized.len()
    );
    assert_eq!(
        serialized.matches(CONTEXT_CLOSE).count(),
        1,
        "an unclosed block is never emitted: exactly one real closing sentinel"
    );
    let marker = "[truncated: source had more than the budget allowed]";
    let marker_at = serialized
        .find(marker)
        .expect("the cut must be visible in the block's own marker");
    let close_at = serialized
        .rfind(CONTEXT_CLOSE)
        .expect("the block must be closed");
    assert!(
        marker_at < close_at,
        "the marker renders inside the delimiters, not as a bare tail"
    );
    assert!(
        !serialized.contains("…[truncated: tool result exceeded"),
        "the loop's own truncation marker must never fire on a fetch result"
    );
    assert!(block.truncated, "the block is flagged for the re-render");
    assert!(block.body.len() < FetchLimits::for_tool_lane().max_total_bytes);
}

/// A multi-byte body is cut on a character boundary: the emitted envelope
/// is valid UTF-8, never a split sequence inside the delimiters.
#[test]
fn the_backstop_cuts_on_a_char_boundary() {
    let mut block = ContextBlock {
        label: "http-fetch example.com".into(),
        body: "é".repeat(FetchLimits::for_tool_lane().max_total_bytes / 2),
        truncated: false,
    };
    let value = bounded_fetch_envelope(
        "https://example.com/page",
        &mut block,
        FetchLimits::tool_lane_cap(),
    )
    .expect("the backstop delivers");
    let content = value["content"].as_str().unwrap();
    assert!(
        content.chars().all(|c| c.is_ascii() || c == 'é'),
        "no replacement characters: the cut never split a sequence"
    );
    assert!(content.ends_with(CONTEXT_CLOSE));
}

/// An absurd final URL that no empty-body block can fit under the cap
/// cannot be delivered: the caller fails the call typed — never an
/// unclosed block.
#[test]
fn an_unfittable_envelope_fails_closed() {
    let mut block = ContextBlock {
        label: "http-fetch example.com".into(),
        body: "body".into(),
        truncated: false,
    };
    assert_eq!(
        bounded_fetch_envelope(
            &format!("https://example.com/{}", "x".repeat(200_000)),
            &mut block,
            65_536
        ),
        None,
        "an envelope that cannot carry even an empty block is not emitted"
    );
}
