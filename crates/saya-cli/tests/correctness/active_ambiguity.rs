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
    let event = stdout
        .lines()
        .find_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value.get("event")?.as_str() == Some("clarification_needed")).then_some(value)
        })
        .unwrap_or_else(|| panic!("no clarification_needed line in: {stdout}"));
    assert_eq!(
        event.pointer("/question").and_then(|q| q.as_str()),
        Some("What defines an active customer?"),
        "the question rides the stream: {stdout}",
    );
    assert_eq!(
        event
            .pointer("/options")
            .and_then(|o| o.as_array())
            .map(Vec::len),
        Some(2),
        "both candidate definitions ride the stream: {stdout}",
    );
    let options: Vec<&str> = event
        .pointer("/options")
        .and_then(|o| o.as_array())
        .unwrap()
        .iter()
        .filter_map(|o| o.as_str())
        .collect();
    assert!(
        options.contains(&"status = 'active'"),
        "the status definition rides the stream: {stdout}",
    );
    assert!(
        options.contains(&"ordered in the last 90 days"),
        "the recency definition rides the stream: {stdout}",
    );
    let _ = std::fs::remove_dir_all(&fixture.root);
}
