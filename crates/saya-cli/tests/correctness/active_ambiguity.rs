//! Ambiguity must ask, not assume.
//!
//! The scripted model calls `request_clarification` with the two candidate
//! definitions of "active". Saya must end the turn (exit 6) with a
//! structured `clarification_needed` event naming both candidates.

use super::common;

#[test]
fn active_customer_ambiguity_requires_input() {
    let fixture = common::build_demo("active-ambiguity");
    let args = serde_json::json!({
        "question": "What defines an active customer?",
        "options": ["status = 'active'", "ordered in the last 90 days"],
    });
    let mut mock = common::spawn_mock(vec![common::tool_call_body(
        "call_ask",
        "request_clarification",
        args,
    )]);
    let output = common::run_ask(&fixture, mock.address(), "count active customers");
    mock.join();
    assert_eq!(
        output.status.code(),
        Some(6),
        "the paused class is exit 6: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(r#""event":"clarification_needed""#),
        "the ndjson stream carries the event: {stdout}",
    );
    assert!(
        stdout.contains("What defines an active customer?"),
        "the question rides the stream: {stdout}",
    );
    assert!(
        stdout.contains("ordered in the last 90 days"),
        "both candidates ride the stream: {stdout}",
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}
