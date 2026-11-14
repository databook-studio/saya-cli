/// A model that read a file once and then batches several edits in one turn
/// carries the same pre-edit digest on every call. The first applies; the
/// later ones are stale only because of this run's own write, so they are
/// re-anchored against the current content instead of being refused.
#[tokio::test]
async fn two_edits_in_one_turn_with_the_pre_edit_digest_both_apply() {
    let sandbox = Sandbox::new("edit-chain-two");
    sandbox
        .ws
        .write("notes.md", b"alpha beta gamma\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let digest = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed")["digest"]
        .as_str()
        .expect("a read returns its digest")
        .to_owned();

    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "alpha",
                "new_text": "ALPHA",
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("the first edit applies against the read state");

    let second = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "gamma",
                "new_text": "GAMMA",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the second edit carries the same pre-edit digest and must re-anchor");

    assert_eq!(second["path"], "notes.md");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("edited file must exist"),
        b"ALPHA beta GAMMA\n",
        "both batched edits land, in order"
    );
}

/// A stale digest outside this run's chain is re-anchored too, when the
/// anchor still resolves exactly once on the current content: the edit
/// applies and the result says the file changed. (Zero or several matches
/// keep the moved-anchor refusal.)
#[tokio::test]
async fn a_stale_digest_outside_this_runs_chain_reanchors_a_unique_anchor() {
    let sandbox = Sandbox::new("edit-chain-foreign-digest");
    sandbox
        .ws
        .write("notes.md", b"alpha beta gamma\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let digest = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed")["digest"]
        .as_str()
        .expect("a read returns its digest")
        .to_owned();
    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "alpha",
                "new_text": "ALPHA",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the first edit applies");

    let second = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "gamma",
                "new_text": "GAMMA",
                "expected_digest": "f".repeat(64),
            }),
        )
        .await
        .expect("a unique anchor on the current content must re-anchor");
    assert!(
        second["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {second}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"ALPHA beta GAMMA\n"
    );
}

/// A change by anyone else between the batched edits is re-anchored when the
/// anchor still matches exactly once on the foreign content: the model's
/// digest was stale, the anchor was unambiguous, and the edit applies to the
/// current content with a note. A foreign change that leaves no unique anchor
/// still refuses (see the moved rows).
#[tokio::test]
async fn a_foreign_change_between_batched_edits_reanchors_a_unique_anchor() {
    let sandbox = Sandbox::new("edit-chain-foreign-change");
    sandbox
        .ws
        .write("notes.md", b"alpha beta gamma\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let digest = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed")["digest"]
        .as_str()
        .expect("a read returns its digest")
        .to_owned();
    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "alpha",
                "new_text": "ALPHA",
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("the first edit applies");

    // A foreign writer replaces the file between the two batched calls.
    sandbox
        .ws
        .write("notes.md", b"foreign alpha gamma\n")
        .expect("foreign overwrite must succeed");

    let second = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "gamma",
                "new_text": "GAMMA",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("a unique anchor on the foreign content must re-anchor");
    assert!(
        second["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {second}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"foreign alpha GAMMA\n"
    );
}

/// A whole-file write carries the same digest precondition. When the current
/// content is this run's own later state and the stale digest is the chain's
/// origin, the write replaces it; a foreign change would still refuse.
#[tokio::test]
async fn a_write_rebased_onto_this_runs_own_edit_applies() {
    let sandbox = Sandbox::new("edit-chain-write");
    sandbox
        .ws
        .write("notes.md", b"alpha beta gamma\n")
        .expect("seed write must succeed");
    let tools = sandbox.tools();
    let digest = tools
        .execute("read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect("the model-facing read must succeed")["digest"]
        .as_str()
        .expect("a read returns its digest")
        .to_owned();
    tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "alpha",
                "new_text": "ALPHA",
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("the edit applies");

    tools
        .execute(
            "write",
            serde_json::json!({
                "path": "notes.md",
                "content": "whole new file\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the write re-anchors onto this run's own edit");
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("written file must exist"),
        b"whole new file\n"
    );
}
