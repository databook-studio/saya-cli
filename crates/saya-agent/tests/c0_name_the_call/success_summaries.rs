use super::*;

/// Property 1 (completion half): a `workspace_write` completion names the
/// file it wrote.
#[tokio::test]
async fn workspace_write_completion_names_the_file_it_wrote() {
    let arguments = serde_json::json!({"path": "notes.md", "content": "hi"});
    let events = run_one(
        workspace_write_tool(),
        arguments,
        &OkExecutor {
            result: serde_json::json!({"path": "notes.md", "bytes_written": 2}),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "notes.md written",
        "workspace_write completion must name the file: got \"{summary}\""
    );
}

/// Property 2 (completion half): a `run_command` completion names the
/// program and its outcome.
#[tokio::test]
async fn run_command_completion_names_the_program_and_its_outcome() {
    let arguments = serde_json::json!({"program": "pytest", "args": ["-q"]});
    let events = run_one(
        run_command_tool(),
        arguments,
        &OkExecutor {
            result: serde_json::json!({"program": "pytest", "exit_code": 1}),
        },
    )
    .await;
    let summary = completed_summary(&events).expect("a ToolCompleted was emitted");
    assert_eq!(
        summary, "pytest exited 1",
        "run_command completion must name the program and outcome: got \"{summary}\""
    );
}
