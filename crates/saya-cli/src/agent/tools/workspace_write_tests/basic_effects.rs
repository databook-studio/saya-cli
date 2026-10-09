/// The refusal that matters most: with `permit_workspace_writes: false` (the
/// default), the loop denies the call before the executor runs — asserted on
/// the filesystem, not just the event stream. If the gate were removed this
/// test goes red because the file would exist.
#[tokio::test]
async fn workspace_write_is_denied_by_the_loop_by_default_and_writes_nothing() {
    let sandbox = Sandbox::new("loop-deny");
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let provider = OneCallProvider {
        call: write_call("notes/out.md", "loop wrote me"),
        turn: std::sync::Mutex::new(0),
    };
    let token = saya_agent::CancellationToken::new();
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_write_definition()],
        AgentLimits::default(),
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: events.clone(),
        },
        token,
    )
    .await
    .expect("denial is not a turn-ending error");
    let denied = events.lock().unwrap().iter().find_map(|event| match event {
        AgentEvent::ToolDenied { name, reason } => Some((name.clone(), reason.clone())),
        _ => None,
    });
    let (name, reason) = denied.expect("a ToolDenied event must be emitted");
    assert_eq!(name, "workspace_write");
    assert!(
        reason.contains("workspace writes are not permitted"),
        "the denial must name the workspace gate, not approval: {reason}"
    );
    assert_eq!(output.tool_metadata[0].status, "denied");
    // The filesystem, not just the event stream: nothing was created.
    assert!(
        !sandbox.ws_root().join("notes").exists(),
        "no directory may be created by a denied call"
    );
    assert!(
        fs::read_dir(sandbox.ws_root())
            .expect("workspace root must still exist")
            .count()
            == 0,
        "the workspace must be untouched after a denied call"
    );
}

/// With the permit granted the loop runs the tool, and the file lands with
/// exactly the bytes given — end to end through the real executor.
#[tokio::test]
async fn workspace_write_runs_in_the_loop_when_writes_are_permitted() {
    let sandbox = Sandbox::new("loop-allow");
    let provider = OneCallProvider {
        call: write_call("notes/out.md", "loop wrote me"),
        turn: std::sync::Mutex::new(0),
    };
    let token = saya_agent::CancellationToken::new();
    let limits = AgentLimits {
        permit_workspace_writes: true,
        ..AgentLimits::default()
    };
    let output = saya_agent::run_agent_with_sink(
        &provider,
        &sandbox.tools(),
        request(),
        vec![workspace_write_definition()],
        limits,
        &AllowReadOnlyApproval,
        &RecordingSink {
            events: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
        },
        token,
    )
    .await
    .expect("run completes");
    assert!(
        !output
            .events
            .iter()
            .any(|event| matches!(event, AgentEvent::ToolDenied { .. })),
        "no denial when workspace writes are permitted"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes/out.md")).expect("the file must exist"),
        b"loop wrote me",
        "the file must hold exactly the bytes the call carried"
    );
}

/// The executor surface: a permitted write lands the exact bytes.
#[tokio::test]
async fn workspace_write_lands_exactly_the_bytes_given() {
    let sandbox = Sandbox::new("exact");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "notes/report.md", "content": "hello workspace"}),
        )
        .await
        .expect("a permitted contained write must succeed");
    assert_eq!(result["path"], "notes/report.md");
    assert_eq!(result["bytes_written"], 15);
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes/report.md")).expect("file must exist"),
        b"hello workspace"
    );
}

/// The harness's mode discipline holds through the tool: the file is 0600 and
/// carries no execute bit.
#[cfg(unix)]
#[tokio::test]
async fn workspace_write_produces_a_0600_file_without_execute_bits() {
    use std::os::unix::fs::PermissionsExt;

    let sandbox = Sandbox::new("mode");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "script.sh", "content": "#!/bin/sh\necho hi\n"}),
        )
        .await
        .expect("a permitted contained write must succeed");
    let mode = fs::metadata(sandbox.ws_root().join("script.sh"))
        .expect("file must exist")
        .permissions()
        .mode();
    assert_eq!(
        mode & 0o777,
        0o600,
        "the file must be owner-read/write only"
    );
    assert_eq!(mode & 0o111, 0, "no execute bit may ever be set");
}

/// A `..` escape is refused with the harness's containment reason, and the
/// sentinel planted outside the workspace is byte-for-byte unchanged.
#[tokio::test]
async fn workspace_write_refuses_a_dot_dot_escape_and_leaves_the_outside_untouched() {
    let sandbox = Sandbox::new("escape");
    fs::write(sandbox.outer.join("outside.txt"), b"sentinel").expect("sentinel plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "../outside.txt", "content": "overwritten"}),
        )
        .await
        .expect_err("a path outside the workspace must be refused, not written");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("escapes the run workspace"),
                "the refusal must name the containment failure: {message}"
            );
            assert!(
                message.contains("../outside.txt"),
                "the refusal must name the refused path: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.outer.join("outside.txt")).expect("sentinel must survive"),
        b"sentinel",
        "nothing may be written outside the workspace"
    );
}

/// An absolute path is refused the same way as an escape — never resolved
/// against the filesystem.
#[tokio::test]
async fn workspace_write_refuses_an_absolute_path() {
    let sandbox = Sandbox::new("absolute");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({"path": "/tmp/saya-should-not-write", "content": "no"}),
        )
        .await
        .expect_err("an absolute path must be refused");
    match error {
        ToolError::WorkspaceWrite(message) => {
            assert!(
                message.contains("escapes the run workspace"),
                "the refusal must name the containment failure: {message}"
            );
        }
        other => panic!("expected a typed workspace-write error, got: {other}"),
    }
}
