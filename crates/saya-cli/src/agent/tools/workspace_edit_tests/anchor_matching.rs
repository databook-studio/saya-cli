/// Zero matches: the tool refuses, names the count, and writes nothing.
#[tokio::test]
async fn zero_matches_writes_nothing() {
    let sandbox = Sandbox::new("zero");
    sandbox
        .ws
        .write("notes.md", b"hello workspace\n")
        .expect("seed write must succeed");
    let before = fs::read(sandbox.ws_root().join("notes.md")).expect("seed file must exist");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "no such anchor anywhere",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("zero matches must refuse, not write");
    let text = error.to_string();
    assert!(
        text.contains("matched 0"),
        "the refusal must name the count, got: {text}"
    );
    assert!(
        text.contains("size:") && text.contains("digest:"),
        "the refusal names the current size and digest so the model can re-anchor: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        before,
        "a refused edit leaves the file byte-identical"
    );
}

/// Multiple matches: the tool refuses with bounded line numbers — never
/// "first wins" — and writes nothing.
#[tokio::test]
async fn multiple_matches_writes_nothing_and_reports_the_lines() {
    let sandbox = Sandbox::new("multi");
    let seed = b"line one: needle here\nline two: filler\nline three: needle here\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "needle here",
                "new_text": "replacement",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("multiple matches must refuse, not pick the first");
    match error {
        saya_agent::ToolError::WorkspaceEditAmbiguous { matches, lines, .. } => {
            assert_eq!(matches, 2, "the refusal must name the count");
            assert_eq!(
                lines,
                vec![1, 3],
                "the refusal reports bounded line numbers"
            );
        }
        other => panic!("expected a typed ambiguity refusal, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed,
        "a refused edit leaves the file byte-identical"
    );
}

/// A moved anchor: an `expected_size`/`expected_digest` mismatch against the
/// file's current state refuses with no write.
#[tokio::test]
async fn a_moved_anchor_refuses_on_the_precondition() {
    let sandbox = Sandbox::new("moved");
    sandbox
        .ws
        .write("notes.md", b"version one: anchor\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    // The model measured a stale size; the file no longer has it.
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_size": 9999,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("a stale size precondition must refuse");
    let text = error.to_string();
    assert!(
        text.contains("changed since measured"),
        "the refusal must name the moved anchor: {text}"
    );
    // The model measured a stale digest; same refusal, no write.
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_digest": "deadbeef",
            }),
        )
        .await
        .expect_err("a stale digest precondition must refuse");
    let text = error.to_string();
    assert!(
        text.contains("changed since measured"),
        "the refusal must name the moved anchor: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"version one: anchor\n",
        "a refused edit leaves the file byte-identical"
    );
}
