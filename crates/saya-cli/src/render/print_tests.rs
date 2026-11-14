//! The `saya -p` render contract: `job_started` first, `job_result` last, and
//! text mode that puts only the answer on stdout.

use super::*;

fn result_value(answer: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "result",
        "subtype": "success",
        "is_error": false,
        "session_id": "p1",
        "job_id": "p1",
        "result": answer,
        "num_turns": 2,
        "duration_ms": 10,
        "duration_api_ms": 5,
        "total_cost_usd": null,
        "usage": {},
        "outcome": {},
        "failure": null,
        "failure_class": null,
        "exit_code": 0,
    })
}

fn started() -> TerminalEvent {
    TerminalEvent::JobStarted {
        job_id: "p1".to_owned(),
        cwd: "/work".to_owned(),
        permission_mode: "read-only".to_owned(),
        write_paths: Vec::new(),
    }
}

/// The first streamed line names the job; the last is the result object, and
/// it carries the CC `type`/`subtype` keys at the top level (flattened, not
/// nested under a `value` key).
#[test]
fn job_result_is_the_last_ndjson_line_and_carries_type_result() {
    let first = render_event(&started(), RenderFormat::Ndjson);
    let answer = render_event(
        &TerminalEvent::JobResult {
            value: result_value("the answer"),
        },
        RenderFormat::Ndjson,
    );
    let stream = format!("{}{}", first.stdout, answer.stdout);
    let lines: Vec<&str> = stream.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "one started line and one result line: {stream}"
    );

    let start: serde_json::Value = serde_json::from_str(lines[0]).expect("started is JSON");
    assert_eq!(start["event"], "job_started");
    assert_eq!(start["job_id"], "p1");

    let result: serde_json::Value = serde_json::from_str(lines[1]).expect("result is JSON");
    assert_eq!(result["event"], "job_result");
    assert_eq!(result["type"], "result");
    assert_eq!(result["subtype"], "success");
    assert_eq!(result["result"], "the answer");
    assert_eq!(result["exit_code"], 0);
    assert!(
        result.get("value").is_none(),
        "the result document is flattened: {result}"
    );
}

/// Text mode: `job <id>` on stderr, the answer on stdout, and a failure line
/// on stderr when the run failed. Nothing else reaches stdout.
#[test]
fn text_mode_prints_only_the_answer_on_stdout() {
    let start = render_event(&started(), RenderFormat::Text);
    assert_eq!(start.stdout, "", "the start line is not stdout");
    assert_eq!(start.stderr, "job p1\n");

    let success = render_event(
        &TerminalEvent::JobResult {
            value: result_value("the answer"),
        },
        RenderFormat::Text,
    );
    assert_eq!(success.stdout, "the answer\n");
    assert_eq!(success.stderr, "", "a clean run states no failure");

    let failed = render_event(
        &TerminalEvent::JobResult {
            value: serde_json::json!({
                "type": "result",
                "subtype": "error_during_execution",
                "is_error": true,
                "result": null,
                "failure": {"class": "provider_error", "message": "the model failed"},
                "failure_class": "provider_error",
                "exit_code": 5,
            }),
        },
        RenderFormat::Text,
    );
    assert_eq!(failed.stdout, "", "a failed run has no answer");
    assert!(
        failed.stderr.contains("job failed: the model failed"),
        "the failure names its message: {:?}",
        failed.stderr
    );
}
