/// A stale digest is resolved against the current content, but the commit
/// re-verifies the digest it resolved against: a writer that changes the file
/// between the tool's read and the write still refuses, and its bytes stand.
#[tokio::test]
async fn a_concurrent_change_between_resolve_and_commit_refuses() {
    let sandbox = Sandbox::new("lenient-commit-race");
    sandbox
        .ws
        .write("notes.md", b"version one: anchor\n")
        .expect("seed write must succeed");
    let target = sandbox.ws_root().join("notes.md");
    let raced = target.clone();
    let tools = sandbox.tools().with_before_commit(Some(Arc::new(move || {
        fs::write(&raced, b"concurrent writer\n").expect("the racing write must land");
    })));

    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("the commit must re-verify the digest it resolved against");
    assert!(
        matches!(error, saya_agent::ToolError::WorkspaceEditMoved { .. }),
        "the race is the typed moved-anchor refusal: {error}"
    );
    assert_eq!(
        fs::read(&target).expect("the racing writer's file must survive"),
        b"concurrent writer\n",
        "the racing writer's bytes are never overwritten"
    );
}
