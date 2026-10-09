/// Replacing an existing file without `expected_digest` refuses before any
/// file access and changes nothing: the digest must be one the model
/// observed, so the refusal names the precondition — never the file's
/// current digest.
#[tokio::test]
async fn a_replace_without_a_digest_changes_nothing() {
    let sandbox = Sandbox::new("no-digest");
    let seed = b"hello workspace\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("notes.md")).expect("seed file must exist");
    let real_digest = observed_digest(&sandbox, "notes.md");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "workspace",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect_err("a replace without a digest must refuse");
    match &error {
        saya_agent::ToolError::WorkspaceEdit(text) => {
            assert!(
                text.contains("expected_digest"),
                "the refusal must name the required argument: {text}"
            );
            assert!(
                text.contains("workspace_read"),
                "the refusal must say where the digest comes from: {text}"
            );
            assert!(
                text.contains("not changed"),
                "the refusal must state the file was not changed: {text}"
            );
        }
        other => panic!("expected the typed missing-digest refusal, got: {other}"),
    }
    let text = error.to_string();
    assert!(
        !text.contains(&real_digest),
        "the refusal must not hand the model the digest it never observed: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        before,
        "a refused edit leaves the file byte-identical"
    );
}

/// A same-size change between the model's observation and the tool call is
/// still caught by the digest precondition: the stale digest mismatches, the
/// refusal is the typed moved-anchor error, and the bytes are untouched.
#[tokio::test]
async fn a_replace_with_a_stale_same_size_digest_is_refused() {
    let sandbox = Sandbox::new("stale-same-size");
    sandbox
        .ws
        .write("notes.md", b"hello world")
        .expect("seed write must succeed");
    let observed = observed_digest(&sandbox, "notes.md");
    // Same size (11 bytes), different bytes — a size guard alone cannot see
    // this; the digest guard must.
    sandbox
        .ws
        .write("notes.md", b"HELLO WORLD")
        .expect("same-size overwrite must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "WORLD",
                "new_text": "SPACE!",
                "expected_digest": observed,
            }),
        )
        .await
        .expect_err("a stale same-size digest must refuse");
    match error {
        saya_agent::ToolError::WorkspaceEditMoved { .. } => {}
        other => panic!("expected a typed moved-anchor refusal, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"HELLO WORLD",
        "a refused edit leaves the file byte-identical"
    );
}

/// The happy path of the new precondition: a digest observed from a fresh
/// read lets the replace proceed and change exactly the intended bytes.
#[tokio::test]
async fn a_replace_with_the_observed_digest_changes_the_intended_bytes() {
    let sandbox = Sandbox::new("observed-digest");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let observed = observed_digest(&sandbox, "notes.md");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
                "expected_digest": observed,
            }),
        )
        .await
        .expect("the observed digest must admit the replace");
    assert_eq!(result["path"], "notes.md");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"hello ANCHOR world\n",
        "the intended bytes change and nothing else"
    );
}
