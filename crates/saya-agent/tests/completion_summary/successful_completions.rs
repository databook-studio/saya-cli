use super::*;

/// A read-only tool that succeeds reports a "read-only" completion.
#[tokio::test]
async fn read_only_tool_reports_read_only_completion() {
    let events = run_one(read_only_tool(), &OkExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("read-only"),
        "a read-only tool reports read-only: got \"{summary}\""
    );
    assert!(
        !summary.contains("write"),
        "a read-only tool must not report a write: got \"{summary}\""
    );
}

/// A write tool that succeeds reports a "local-state write" completion, *not*
/// "read-only" — the fix at the heart of §4.
#[tokio::test]
async fn write_tool_does_not_report_read_only_completion() {
    let events = run_one(write_tool(), &OkExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("write"),
        "a write tool reports a write: got \"{summary}\""
    );
    assert!(
        !summary.contains("read-only"),
        "a write tool must not report read-only — that would overstate: got \"{summary}\""
    );
}

/// A tool declaring its own `completion` reports that text — not a generic
/// label that would be false for what the tool actually did.
#[tokio::test]
async fn custom_completion_is_reported_verbatim() {
    let events = run_one(custom_completion_tool(), &OkExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "chart written and opened",
        "the definition's own completion text is reported: got \"{summary}\""
    );
    assert!(
        !summary.contains("read-only database tool"),
        "a tool that writes a file and opens a browser must not be labelled \
         a read-only database tool: got \"{summary}\""
    );
}

/// `read_only` feeds only the completion labels, never the execution gates —
/// gating keys on the declared *effect*, which is why an approver that permits
/// the effect runs both shapes alike. Two tools identical except `read_only`
/// therefore execute identically.
#[tokio::test]
async fn read_only_does_not_gate_execution() {
    for read_only in [true, false] {
        let tool = ToolDefinition {
            read_only,
            ..custom_completion_tool()
        };
        let events = run_one(tool, &OkExecutor).await;
        assert!(
            completed_summary(&events).is_some(),
            "read_only = {read_only}: the tool executed and completed"
        );
    }
}
