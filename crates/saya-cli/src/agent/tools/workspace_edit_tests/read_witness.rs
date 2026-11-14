/// A completed model-facing read supplies the omitted edit precondition for
/// this same toolset, without copying the digest through the model payload.
#[tokio::test]
async fn a_read_witness_allows_an_anchored_edit_without_a_copied_digest() {
    let sandbox = Sandbox::new("read-witness");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let read = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");
    assert_eq!(read["content"], "     1→hello anchor world\n");

    let edited = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect("the toolset's completed read should guard this edit");
    assert_eq!(edited["path"], "notes.md");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("edited file must exist"),
        b"hello ANCHOR world\n"
    );
}

#[tokio::test]
async fn a_read_witness_does_not_replace_mistyped_explicit_preconditions() {
    let sandbox = Sandbox::new("read-witness-invalid-preconditions");
    let original = b"hello anchor world\n";
    sandbox
        .ws
        .write("notes.md", original)
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");

    for (name, arguments, expected) in [
        (
            "write",
            serde_json::json!({
                "path": "notes.md",
                "content": "replacement",
                "expected_digest": 7
            }),
            saya_agent::ToolError::ExpectedDigestNotString,
        ),
        (
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "changed",
                "expected_digest": 7
            }),
            saya_agent::ToolError::ExpectedDigestNotString,
        ),
        (
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "changed",
                "expected_size": "wrong"
            }),
            saya_agent::ToolError::ExpectedSizeNotUint,
        ),
    ] {
        let error = tools
            .execute(name, arguments)
            .await
            .expect_err("non-null mistyped preconditions must not use the witness");
        assert_eq!(error, expected);
        assert_eq!(
            fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
            original,
            "an invalid explicit precondition leaves bytes unchanged"
        );
    }
}

/// An explicit wrong digest is never replaced by the witness, but a stale
/// explicit digest is re-anchored when the anchor resolves exactly once on
/// the current content: the edit applies with a note.
#[tokio::test]
async fn an_explicit_wrong_digest_reanchors_when_the_anchor_is_unique() {
    let sandbox = Sandbox::new("read-witness-explicit-digest");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");
    sandbox
        .ws
        .write("notes.md", b"HELLO anchor world\n")
        .expect("same-size external overwrite must succeed");
    let explicit = "f".repeat(64);

    let edited = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "changed",
                "expected_digest": explicit
            }),
        )
        .await
        .expect("a unique anchor on the current content must re-anchor");
    assert!(
        edited["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {edited}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"HELLO changed world\n"
    );
}

/// A file that changed after the read is re-anchored when the anchor still
/// resolves exactly once: the edit applies to the current content, and the
/// note says why. The stale digest is never silently treated as current.
#[tokio::test]
async fn a_stale_read_witness_reanchors_a_unique_anchor_with_a_note() {
    let sandbox = Sandbox::new("read-witness-stale");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");
    sandbox
        .ws
        .write("notes.md", b"HELLO anchor world\n")
        .expect("same-size external overwrite must succeed");

    let edited = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "changed"
            }),
        )
        .await
        .expect("a unique anchor on the current content must re-anchor");
    assert!(
        edited["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {edited}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"HELLO changed world\n"
    );
}

/// A read anywhere earlier in the run admits a later edit. The loop
/// settles the invocation at a pause boundary and again when the run ends;
/// the witness survives that boundary, so the model never re-reads a file it
/// has already read this run.
#[tokio::test]
async fn a_read_survives_the_invocation_boundary_and_admits_a_later_edit() {
    let sandbox = Sandbox::new("read-witness-later-turn");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");
    tools
        .settle_invocation()
        .await
        .expect("the owned read worker must settle");

    let edited = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect("a read earlier in the run must admit this edit");
    assert_eq!(edited["path"], "notes.md");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("edited file must exist"),
        b"hello ANCHOR world\n"
    );
}

/// A failed edit leaves the witness in place — an anchor miss leaves the file
/// byte-identical, so the observation the model made still holds and the next
/// attempt needs no re-read.
#[tokio::test]
async fn a_failed_edit_keeps_the_read_witness_for_the_next_attempt() {
    let sandbox = Sandbox::new("read-witness-failed-edit");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");

    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "no such anchor",
                "new_text": "x",
            }),
        )
        .await
        .expect_err("an anchor that matches nothing must refuse");

    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect("a failed edit must not clear the witness");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"hello ANCHOR world\n"
    );
}

/// A failed write leaves the witness in place too — the model's earlier read
/// still admits the next edit without a re-read.
#[tokio::test]
async fn a_failed_write_keeps_the_read_witness_for_the_next_edit() {
    let sandbox = Sandbox::new("read-witness-failed-write");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");

    tools
        .execute(
            "write",
            serde_json::json!({
                "path": "notes.md",
                "content": "clobber",
                "expected_digest": "f".repeat(64),
            }),
        )
        .await
        .expect_err("a wrong explicit digest must refuse the replace");

    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "ANCHOR",
            }),
        )
        .await
        .expect("a failed write must not clear the witness");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"hello ANCHOR world\n"
    );
}

#[tokio::test]
async fn rebinding_a_workspace_clears_the_previous_roots_witness() {
    let sandbox = Sandbox::new("read-witness-rebind");
    sandbox
        .ws
        .write("notes.md", b"hello anchor world\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed");

    let other_root = sandbox.outer.join("other-ws");
    fs::create_dir_all(&other_root).expect("second workspace root must create");
    fs::write(other_root.join("notes.md"), b"hello anchor world\n")
        .expect("second fixture must seed");
    let rebound = tools.with_workspace(Some(Arc::new(
        saya_harness::workspace::Workspace::open(&other_root).expect("second workspace must open"),
    )));
    let error = rebound
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "anchor",
                "new_text": "changed"
            }),
        )
        .await
        .expect_err("a workspace rebind must not carry the prior root's witness");

    assert!(matches!(
        error,
        saya_agent::ToolError::WorkspaceDigestRequired { .. }
    ));
    assert_eq!(
        fs::read(other_root.join("notes.md")).expect("second file must survive"),
        b"hello anchor world\n"
    );
}
