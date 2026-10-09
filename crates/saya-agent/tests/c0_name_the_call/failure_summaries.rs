use super::*;

/// Property 3: a failed call is still classified as failed — the substring
/// contract holds. Both the status derivation (`contains("failed")`) and the
/// summary text pin it, for both covered tools.
#[tokio::test]
async fn a_failed_call_is_still_classified_as_failed() {
    for (tool, arguments) in [
        (
            workspace_write_tool(),
            serde_json::json!({"path": "notes.md", "content": "hi"}),
        ),
        (
            run_command_tool(),
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
        ),
    ] {
        let events = Arc::new(Mutex::new(Vec::new()));
        let provider = OneCallProvider {
            call: ToolCall {
                id: "c1".into(),
                name: tool.name.clone(),
                arguments: arguments.clone(),
            },
            turn: Mutex::new(0),
        };
        let sink = RecordingSink {
            events: events.clone(),
        };
        let limits = AgentLimits {
            permit_candidate_writes: true,
            permit_workspace_writes: true,
            permit_external_effects: true,
            ..AgentLimits::default()
        };
        let output = run_agent_with_sink(
            &provider,
            &FailExecutor,
            request(),
            vec![tool.clone()],
            limits,
            &ApproveAll,
            &sink,
            saya_agent::CancellationToken::new(),
        )
        .await
        .expect("run completes");
        let events = events.lock().unwrap();
        let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
        assert!(
            summary.contains("failed"),
            "{} failure summary must keep \"failed\": got \"{summary}\"",
            tool.name
        );
        assert_eq!(
            tool_status(&output),
            "failed",
            "{} failure status must still read failed",
            tool.name
        );
    }
}

/// Property 4 (completion half): a failure line carries no success verb.
/// A failed call must not reuse the success completion text at all — no
/// "written", no "ran", no "listed" — for either covered tool. The bounded,
/// redacted error reason (Phase 7 packet 2) may follow the key fact after
/// " — ", but the success text itself never appears.
#[tokio::test]
async fn a_failure_line_carries_no_success_verb() {
    for (tool, arguments, success_text, expected_prefix) in [
        (
            workspace_write_tool(),
            serde_json::json!({"path": "notes.md", "content": "hi"}),
            "workspace file written",
            "failed notes.md",
        ),
        (
            run_command_tool(),
            serde_json::json!({"program": "pytest", "args": ["-q"]}),
            "host command ran",
            "failed pytest",
        ),
    ] {
        let events = run_one(tool.clone(), arguments, &FailExecutor).await;
        let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
        assert!(
            summary.starts_with(expected_prefix),
            "{} failure summary must name the call without the success text: got \"{summary}\"",
            tool.name
        );
        for verb in ["written", "ran", "listed"] {
            assert!(
                !summary.contains(verb),
                "{} failure summary must not contain the success verb \"{verb}\": got \"{summary}\"",
                tool.name
            );
        }
        assert!(
            !summary.contains(success_text),
            "{} failure summary must not carry the success completion text: got \"{summary}\"",
            tool.name
        );
    }
}

/// Property 5 (completion half): a tool this slice does not cover keeps
/// today's text, byte-exact.
#[tokio::test]
async fn an_uncovered_tool_keeps_todays_text_byte_exact() {
    let events = run_one(
        uncovered_tool(),
        serde_json::json!({}),
        &OkExecutor {
            result: serde_json::json!({"entries": []}),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "workspace directory listed",
        "an uncovered tool must keep today's text byte-exact: got \"{summary}\""
    );
    let events = run_one(uncovered_tool(), serde_json::json!({}), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.starts_with("failed to complete: workspace directory listed"),
        "an uncovered tool must keep today's failure text as the prefix: got \"{summary}\""
    );
}

/// Extra: the completion line never carries tool output — stdout/stderr and
/// file content stay out of the summary; only the typed key facts name it.
#[tokio::test]
async fn completion_detail_never_carries_tool_output() {
    let events = run_one(
        run_command_tool(),
        serde_json::json!({"program": "pytest"}),
        &OkExecutor {
            result: serde_json::json!({
                "program": "pytest",
                "exit_code": 0,
                "stdout": {"text": "SECRET-STDOUT"},
                "stderr": {"text": "SECRET-STDERR"},
            }),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(summary, "pytest exited 0");
    assert!(
        !summary.contains("SECRET-STDOUT") && !summary.contains("SECRET-STDERR"),
        "tool output must not reach the line: got \"{summary}\""
    );
}
