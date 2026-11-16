use super::*;

/// A write tool that fails reports a write failure whose summary keeps the
/// "failed" substring, so `tool_metadata.status` still reads "failed".
#[tokio::test]
async fn write_tool_failure_summary_keeps_the_failed_substring() {
    let events = run_one(write_tool(), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("write"),
        "a failed write tool still names a write: got \"{summary}\""
    );
    assert!(
        summary.contains("failed"),
        "the failure summary keeps \"failed\" so tool_metadata reads failed: got \"{summary}\""
    );
}

/// A read-only tool that fails reports a read-only failure (the read-only
/// direction still holds on the failure path).
#[tokio::test]
async fn read_only_tool_failure_reports_read_only_failure() {
    let events = run_one(read_only_tool(), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("read-only"),
        "a failed read-only tool still reports read-only: got \"{summary}\""
    );
    assert!(
        summary.contains("failed"),
        "the failure summary keeps \"failed\": got \"{summary}\""
    );
}

/// The failure summary for a custom-completion tool still keeps the
/// "failed" substring — `tool_metadata.status` and the statement-outcome
/// memory both key on it, so a custom completion must not break the
/// failure signal.
#[tokio::test]
async fn custom_completion_failure_keeps_the_failed_substring() {
    let events = run_one(custom_completion_tool(), &FailExecutor).await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert!(
        summary.contains("failed"),
        "the failure summary keeps \"failed\" so tool_metadata reads failed: got \"{summary}\""
    );
}
