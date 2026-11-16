/// All anchors in a multi-edit resolve against the same original snapshot.
/// The second anchor therefore still matches `B` after the first replaces
/// original `A` with `B`.
#[tokio::test]
async fn multi_edit_resolves_each_anchor_against_the_original_file() {
    let sandbox = Sandbox::new("multi-edit-original");
    sandbox
        .ws
        .write("notes.md", b"A\nB\n")
        .expect("seed write must succeed");
    let digest = observed_digest(&sandbox, "notes.md");
    let tools = sandbox.tools();

    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "expected_digest": digest,
                "edits": [
                    {"old_text": "A", "new_text": "B"},
                    {"old_text": "B", "new_text": "C"}
                ]
            }),
        )
        .await
        .expect("both unique anchors should be applied as one edit");
    assert_eq!(result["path"], "notes.md");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("edited file must exist"),
        b"B\nC\n"
    );
}

#[tokio::test]
async fn multi_edit_rejects_a_late_missing_anchor_without_publishing_earlier_edits() {
    let sandbox = Sandbox::new("multi-edit-late-miss");
    let original = b"A\nB\n";
    sandbox
        .ws
        .write("notes.md", original)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
                "edits": [
                    {"old_text": "A", "new_text": "X"},
                    {"old_text": "missing", "new_text": "Y"}
                ]
            }),
        )
        .await
        .expect_err("every anchor must validate before publication");
    assert!(matches!(
        error,
        saya_agent::ToolError::WorkspaceEditNoMatch { .. }
    ));
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        original
    );
}

#[tokio::test]
async fn multi_edit_counts_overlapping_anchor_occurrences_as_ambiguous() {
    let sandbox = Sandbox::new("multi-edit-overlap");
    let original = b"aaa";
    sandbox
        .ws
        .write("notes.md", original)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
                "edits": [{"old_text": "aa", "new_text": "b"}]
            }),
        )
        .await
        .expect_err("the overlapping occurrences must not be treated as one match");
    assert!(matches!(
        error,
        saya_agent::ToolError::WorkspaceEditAmbiguous { matches: 2, .. }
    ));
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        original
    );
}

#[tokio::test]
async fn overlapping_edit_pairs_refuse_atomically() {
    let sandbox = Sandbox::new("multi-edit-pair-overlap");
    let original = b"abc";
    sandbox
        .ws
        .write("notes.md", original)
        .expect("seed write must succeed");
    let error = sandbox
        .tools()
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "expected_digest": observed_digest(&sandbox, "notes.md"),
                "edits": [
                    {"old_text": "ab", "new_text": "X"},
                    {"old_text": "bc", "new_text": "Y"}
                ]
            }),
        )
        .await
        .expect_err("overlapping original ranges must refuse before publication");
    assert_eq!(
        error,
        saya_agent::ToolError::WorkspaceEdit(super::workspace::MULTI_EDIT_OVERLAP_REFUSAL.into())
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        original
    );
}
