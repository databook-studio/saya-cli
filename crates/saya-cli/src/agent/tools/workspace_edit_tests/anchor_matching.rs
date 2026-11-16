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
            "edit",
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
            "edit",
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

/// A stale `expected_size` alone no longer refuses once the digest and the
/// anchor agree: the digest is the content guard, and the anchor resolves
/// exactly once on the file the tool actually read.
#[tokio::test]
async fn a_stale_expected_size_alone_does_not_refuse_a_unique_anchor() {
    let sandbox = Sandbox::new("moved-size");
    sandbox
        .ws
        .write("notes.md", b"version one: anchor\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_size": 9999,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect("a stale size with the current digest and a unique anchor applies");
    assert!(result.get("note").is_none(), "no re-anchor note is needed");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"version one: replacement\n"
    );
}

/// A stale digest with a unique anchor re-anchors onto the current content;
/// a digest with zero matching anchors keeps the typed moved-anchor refusal
/// and writes nothing.
#[tokio::test]
async fn a_moved_digest_reanchors_a_unique_anchor_and_refuses_a_missing_one() {
    let sandbox = Sandbox::new("moved-digest");
    sandbox
        .ws
        .write("notes.md", b"version one: anchor\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_digest": "deadbeef",
            }),
        )
        .await
        .expect("a unique anchor on the current content must re-anchor");
    assert!(
        result["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {result}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"version one: replacement\n"
    );

    // The anchor is gone now: the same stale digest refuses, unchanged.
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "replacement",
                "expected_digest": "deadbeef",
            }),
        )
        .await
        .expect_err("a stale digest with no matching anchor must refuse");
    assert!(matches!(
        error,
        saya_agent::ToolError::WorkspaceEditMoved { .. }
    ));
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        b"version one: replacement\n",
        "a refused edit leaves the file byte-identical"
    );
}
