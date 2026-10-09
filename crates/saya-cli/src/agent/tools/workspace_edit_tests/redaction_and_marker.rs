/// An edit that would replace the real value with the literal `[redacted]`
/// marker is refused — the same guard `workspace_write` applies — and the
/// file on disk keeps the real value.
#[tokio::test]
async fn workspace_edit_refuses_to_add_redaction_placeholders() {
    let sandbox = Sandbox::new("redaction-guard");
    let seed = b"a = f(token=real_value)\n";
    sandbox
        .ws
        .write("a.py", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "a.py",
                "old_text": "token=real_value",
                "new_text": "token=[redacted]",
                "expected_digest": observed_digest(&sandbox, "a.py"),
            }),
        )
        .await
        .expect_err("an edit that adds a redaction placeholder must be refused");
    let text = error.to_string();
    assert!(
        text.contains("[redacted]"),
        "the refusal must name the marker: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("a.py")).expect("file must survive"),
        seed,
        "a refused edit leaves the real value on disk untouched"
    );
}

/// A file that already holds the marker (e.g. an earlier legitimate write)
/// can still be edited as long as the edit does not grow the count: the
/// guard is a growth check, not a ban on the literal text.
#[tokio::test]
async fn a_file_that_already_holds_the_marker_can_still_be_edited() {
    let sandbox = Sandbox::new("marker-stays-flat");
    let seed = b"status: [redacted]\nother: line\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "other: line",
                "new_text": "other: replaced",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("the count stays at 1, so the edit must be allowed");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"status: [redacted]\nother: replaced\n".to_vec(),
        "the edit must land; the pre-existing marker is untouched"
    );
}
