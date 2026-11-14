//! Provider usage in the human-readable job view.

use super::*;

fn show(usage: serde_json::Value) -> String {
    render_coding_result(
        &CodingCommand::Show { id: "job-1".into() },
        &json!({"job_id":"job-1","state":"completed","failure":null,"usage":usage,
            "next_cursor":null,"children":[]}),
    )
}

fn show_with_progress(progress: serde_json::Value) -> String {
    render_coding_result(
        &CodingCommand::Show { id: "job-1".into() },
        &json!({"job_id":"job-1","state":"completed","failure":null,
            "usage":serde_json::Value::Null,"progress":progress,"next_cursor":null,"children":[]}),
    )
}

#[test]
fn show_reports_provider_usage_totals_and_breakdowns() {
    let text = show(json!({"source":"durable_transport_spans","complete":true,
        "requests":4,"input":80000,"cached_input":76000,"output":4000,"reasoning":2000}));
    assert!(
        text.contains("Usage: 4 requests, 80000 input (76000 cached), 4000 output, 2000 reasoning"),
        "{text}"
    );
}

#[test]
fn show_marks_partial_usage_and_omits_unreported_breakdowns() {
    let text = show(json!({"source":"durable_transport_spans","complete":false,
        "requests":1,"input":10,"cached_input":null,"output":5}));
    assert!(
        text.contains("Usage: 1 request, 10 input, 5 output (incomplete)"),
        "{text}"
    );
}

#[test]
fn show_reports_the_model_wall_time_the_spans_measured() {
    let text = show(json!({"source":"durable_transport_spans","complete":true,
        "requests":1,"input":10,"cached_input":null,"output":5,
        "model_wall_time_ms":1234}));
    assert!(text.contains("1234 ms model"), "{text}");
}

#[test]
fn show_reports_a_provider_reported_cost() {
    let text = show(json!({"source":"durable_transport_spans","complete":true,
        "requests":1,"input":10,"cached_input":null,"output":5,"cost_usd":0.0125}));
    assert!(text.contains("$0.0125"), "{text}");
}

#[test]
fn show_reports_the_recorded_progress() {
    let text = show_with_progress(json!({"turns":3,"last_progress_turn":2,
        "progress_events":{"new_read":1,"workspace_change":1}}));
    assert!(
        text.contains("Progress: 3 turns, 2 events, last at turn 2"),
        "{text}"
    );
}

#[test]
fn show_reports_the_latest_progress_check() {
    let text = show_with_progress(json!({"turns":8,"last_progress_turn":1,
        "progress_events":{"workspace_change":1},
        "check":{"message":"no progress for 3 of 8 turns"}}));
    assert!(
        text.contains("Progress: 8 turns, 1 events, last at turn 1 — no progress for 3 of 8 turns"),
        "{text}"
    );
}

#[test]
fn show_says_nothing_about_usage_when_no_request_reported_it() {
    let text = show(json!({"source":"durable_transport_spans","complete":true,
        "requests":0,"input":0,"cached_input":null,"output":0}));
    assert!(!text.contains("Usage:"), "{text}");
}
