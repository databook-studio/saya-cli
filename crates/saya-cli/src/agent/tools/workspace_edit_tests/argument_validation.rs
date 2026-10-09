/// Unknown arguments are rejected before anything else runs.
#[tokio::test]
async fn workspace_edit_rejects_unknown_arguments() {
    let sandbox = Sandbox::new("unknown-args");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "sql": "SELECT 1",
            }),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, saya_agent::ToolError::UnsupportedProperty);
}

/// A non-string `old_text` is a typed error of its own, distinct from the
/// path's, so the model can fix the right argument. Same for `new_text` and
/// the optional `expected_*` precondition.
#[tokio::test]
async fn workspace_edit_rejects_mistyped_arguments() {
    let sandbox = Sandbox::new("arg-types");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({"path": "notes.md", "old_text": 7, "new_text": "b"}),
        )
        .await
        .expect_err("a non-string old_text must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::OldTextNotString);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({"path": "notes.md", "old_text": "a", "new_text": 7}),
        )
        .await
        .expect_err("a non-string new_text must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::NewTextNotString);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "expected_size": "huge",
            }),
        )
        .await
        .expect_err("a non-integer expected_size must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ExpectedSizeNotUint);
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "a",
                "new_text": "b",
                "expected_digest": 7,
            }),
        )
        .await
        .expect_err("a non-string expected_digest must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ExpectedDigestNotString);
}

/// A non-UTF-8 target is refused loudly: reads are lossy, so a byte-exact
/// anchor cannot be trusted. Binary support is a later slice, not this one.
#[tokio::test]
async fn workspace_edit_refuses_a_non_utf8_target() {
    let sandbox = Sandbox::new("non-utf8");
    sandbox
        .ws
        .write("blob.bin", &[0x66, 0x6f, 0xff, 0xfe, 0x6f])
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "blob.bin",
                "old_text": "fo",
                "new_text": "bar",
                "expected_digest": observed_digest(&sandbox, "blob.bin"),
            }),
        )
        .await
        .expect_err("a non-UTF-8 target must be refused");
    assert_eq!(
        error,
        saya_agent::ToolError::WorkspaceEditNotText {
            path: "blob.bin".into(),
        }
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("blob.bin")).expect("file must survive"),
        vec![0x66, 0x6f, 0xff, 0xfe, 0x6f],
        "a refused edit leaves the file byte-identical"
    );
}

/// A `..` escape is refused with the harness's containment reason, and the
/// sentinel planted outside the workspace is byte-for-byte unchanged.
#[tokio::test]
async fn workspace_edit_refuses_a_dot_dot_escape() {
    let sandbox = Sandbox::new("escape");
    fs::write(sandbox.outer.join("outside.txt"), b"sentinel").expect("sentinel plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "../outside.txt",
                "old_text": "sentinel",
                "new_text": "overwritten",
                "expected_digest": "placeholder-digest-outside-the-workspace",
            }),
        )
        .await
        .expect_err("a path outside the workspace must be refused, not edited");
    let text = error.to_string();
    assert!(
        text.contains("escapes the run workspace"),
        "the refusal must name the containment failure: {text}"
    );
    assert_eq!(
        fs::read(sandbox.outer.join("outside.txt")).expect("sentinel must survive"),
        b"sentinel",
        "nothing may be written outside the workspace"
    );
}
