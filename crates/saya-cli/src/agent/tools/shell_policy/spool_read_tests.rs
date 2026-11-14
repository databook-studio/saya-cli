//! The job's own shell-output spool is readable by its own `read` and `grep`,
//! and by nothing else: the elision marker names a path the model can open,
//! while another session's spool directory stays outside every tool's reach.

use std::fs;
use std::sync::Arc;

use saya_agent::{ApprovalPolicy, CancellationToken, ToolExecutor};
use saya_harness::runner::HostPolicy;
use saya_harness::workspace::Workspace;
use serde_json::json;

use super::{Background, compose_host_only, spool_dir};
use crate::agent::tools::DatabaseTools;

/// A shell over `root` whose oversized results save under `state_dir`.
fn shell_with_spool(
    root: &std::path::Path,
    state_dir: &std::path::Path,
) -> Option<super::ShellTool> {
    compose_host_only(
        root,
        ApprovalPolicy::Bypass,
        None,
        HostPolicy::default(),
        CancellationToken::new(),
        Background::Off,
        Some(state_dir),
        saya_config::CommandRules::default(),
    )
}

fn sandbox(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!("saya-spool-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("ws")).expect("workspace must create");
    root
}

/// Runs one oversized command and returns the elision marker's saved path.
async fn saved_path(shell: &super::ShellTool) -> String {
    let result = shell
        .execute(
            "shell",
            json!({
                "command": "awk 'BEGIN { for (i = 0; i < 40000; i++) printf \"x\"; \
                            printf \"\\nTAIL_MARKER\\n\" }'",
                "sandbox": false,
            }),
        )
        .await
        .expect("the oversized shell call runs");
    assert_eq!(result["exit_code"], 0, "{result}");
    assert_eq!(result["truncated"], true, "the inline bound cut the result");
    let stdout = result["stdout"].as_str().expect("stdout is a string");
    assert!(
        stdout.contains("bytes elided; full output saved to "),
        "the marker names the saved output: {stdout}"
    );
    assert!(
        stdout.contains("; read it with read/grep] ..."),
        "the marker says how to open it: {stdout}"
    );
    let start = stdout.find("saved to ").expect("the marker names a path") + "saved to ".len();
    let end = start
        + stdout[start..]
            .find("; read it with")
            .expect("the marker ends the path");
    stdout[start..end].to_owned()
}

#[tokio::test]
async fn the_jobs_own_spool_is_readable_by_read_and_grep() {
    let root = sandbox("readable");
    let state = root.join("state");
    fs::create_dir_all(&state).expect("state directory must create");
    let shell = shell_with_spool(&root.join("ws"), &state).expect("a host shell composes");
    let path = saved_path(&shell).await;

    let tools = DatabaseTools::for_child_checkout(Arc::new(
        Workspace::open(&root.join("ws")).expect("workspace must open"),
    ))
    .with_spool_dir(Some(spool_dir(&state)));

    let read = tools
        .execute("read", json!({"path": path}))
        .await
        .expect("the model can read the path the marker named");
    assert_eq!(read["path"], path.as_str());
    let content = read["content"].as_str().expect("read serves text");
    assert!(
        content.contains("TAIL_MARKER"),
        "the whole saved output is readable: {content}"
    );
    assert!(
        read["truncated"] != true,
        "the saved output fits the existing read bound: {read}"
    );

    let grep = tools
        .execute("grep", json!({"pattern": "TAIL_MARKER", "path": path}))
        .await
        .expect("the model can grep the path the marker named");
    assert_eq!(grep["matches_found"], 1, "{grep}");
    let matches = grep["matches"].as_array().expect("matches is an array");
    assert_eq!(matches.len(), 1, "{grep}");
    assert_eq!(
        matches[0]["path"],
        path.as_str(),
        "a spool match is named by the absolute path read accepts"
    );

    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn another_sessions_spool_is_not_readable() {
    let root = sandbox("private");
    let own_state = root.join("own-state");
    let other_state = root.join("other-state");
    fs::create_dir_all(&own_state).expect("own state must create");
    let other_dir = spool_dir(&other_state);
    fs::create_dir_all(&other_dir).expect("other spool must create");
    let other_file = other_dir.join("shell-1-1.log");
    fs::write(&other_file, b"another session's output\n").expect("other spool file must write");

    let tools = DatabaseTools::for_child_checkout(Arc::new(
        Workspace::open(&root.join("ws")).expect("workspace must open"),
    ))
    .with_spool_dir(Some(spool_dir(&own_state)));

    let path = other_file.display().to_string();
    let read = tools
        .execute("read", json!({"path": path}))
        .await
        .expect_err("another session's spool file is outside this tool's reach");
    assert!(
        !read.to_string().is_empty(),
        "the refusal names the containment failure"
    );
    let grep = tools
        .execute("grep", json!({"pattern": "another", "path": path}))
        .await
        .expect_err("another session's spool directory is not searchable");
    assert!(!grep.to_string().is_empty());

    let _ = fs::remove_dir_all(root);
}
