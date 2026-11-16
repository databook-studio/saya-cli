/// `replace_all` splices every non-overlapping occurrence against the one
/// original snapshot and publishes the result as exactly one commit: one
/// checkpoint, one before-image, one digest. That is what makes a rename
/// across one file one call.
#[tokio::test]
async fn replace_all_replaces_every_occurrence_and_publishes_once() {
    let sandbox = Sandbox::new("replace-all");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] =
        b"needle one\nfiller\nneedle two\nneedle three\nneedle four\nneedle five\n";
    sandbox
        .ws
        .write("notes.md", original)
        .expect("seed write must succeed");
    let digest = observed_digest(&sandbox, "notes.md");

    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "needle",
                "new_text": "thread",
                "replace_all": true,
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("replace_all must replace every occurrence");
    assert_eq!(result["path"], "notes.md");
    assert_eq!(result["replacements"], 5, "the result reports the count");
    assert_eq!(result["bytes_replaced"], 5 * "needle".len());
    assert_eq!(result["bytes_written"], 5 * "thread".len());

    let edited = fs::read(sandbox.ws_root().join("notes.md")).expect("edited file must exist");
    assert_eq!(
        edited,
        b"thread one\nfiller\nthread two\nthread three\nthread four\nthread five\n".as_slice(),
        "every occurrence is replaced; nothing else moves"
    );
    assert_eq!(result["size"], edited.len() as u64);
    assert_eq!(
        result["digest"],
        observed_digest(&sandbox, "notes.md"),
        "the reported digest describes the file the one commit published"
    );

    let records = store.list_checkpoints(SESSION).expect("list checkpoints");
    assert_eq!(
        records.len(),
        1,
        "the whole replace_all is one publication, not one per occurrence"
    );
    let record = &records[0];
    assert_eq!(record.state, CheckpointState::Committed);
    assert_eq!(record.source, EditSource::WorkspaceEdit);
    assert_eq!(record.path, "notes.md");
    assert_eq!(
        record.before,
        BeforeImage::Present(saya_types::FileVersion {
            sha256: digest,
            size: original.len() as u64,
        }),
        "the one before-image is the original file, not a partial splice"
    );
    let image = store
        .read_before_image(SESSION, record.id.as_str())
        .expect("the before-image reads back");
    assert_eq!(image, original);
}

/// `replace_all: false` — the default — keeps today's exactly-once refusal
/// and writes nothing.
#[tokio::test]
async fn replace_all_false_with_five_occurrences_still_refuses() {
    let sandbox = Sandbox::new("replace-all-off");
    let seed = b"needle\nneedle\nneedle\nneedle\nneedle\n";
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
                "old_text": "needle",
                "new_text": "thread",
                "replace_all": false,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("an explicit false keeps the exactly-once refusal");
    match error {
        saya_agent::ToolError::WorkspaceEditAmbiguous { matches, .. } => {
            assert_eq!(matches, 5, "the refusal names the count");
        }
        other => panic!("expected the existing ambiguity refusal, got: {other}"),
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed,
        "a refused edit leaves the file byte-identical"
    );
}

/// `replace_all` names the replace form only: combining it with `edits` or
/// with `offset`+`chunk` is a typed refusal, not a guess.
#[tokio::test]
async fn replace_all_with_another_edit_shape_is_a_typed_refusal() {
    let sandbox = Sandbox::new("replace-all-conflict");
    let seed = b"needle\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();

    for arguments in [
        serde_json::json!({
            "path": "notes.md",
            "edits": [{"old_text": "needle", "new_text": "thread"}],
            "replace_all": true,
        }),
        serde_json::json!({
            "path": "notes.md",
            "offset": 0,
            "chunk": "more",
            "replace_all": true,
        }),
    ] {
        let error = tools
            .execute("edit", arguments.clone())
            .await
            .expect_err("two edit shapes at once must refuse");
        assert_eq!(
            error,
            saya_agent::ToolError::ReplaceAllConflict,
            "the refusal names the conflicting argument for: {arguments}"
        );
    }
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed
    );
}

/// A non-boolean `replace_all` is its own typed argument error, so the model
/// fixes that argument rather than the anchor.
#[tokio::test]
async fn replace_all_must_be_a_boolean() {
    let sandbox = Sandbox::new("replace-all-type");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "needle",
                "new_text": "thread",
                "replace_all": "yes",
            }),
        )
        .await
        .expect_err("a non-boolean replace_all must be rejected at validation");
    assert_eq!(error, saya_agent::ToolError::ReplaceAllNotBool);
}

/// A stale digest re-anchors under `replace_all` when the count rule still
/// holds — at least one occurrence — and the whole call lands once, with a
/// note that the file changed.
#[tokio::test]
async fn replace_all_reanchors_a_stale_digest_with_matches() {
    let sandbox = Sandbox::new("replace-all-stale");
    let seed = b"needle\nneedle\nneedle\nneedle\nneedle\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let tools = sandbox.tools();

    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "needle",
                "new_text": "thread",
                "replace_all": true,
                "expected_digest": "deadbeef",
            }),
        )
        .await
        .expect("a stale digest with matching anchors must re-anchor");
    assert_eq!(result["replacements"], 5);
    assert!(
        result["note"]
            .as_str()
            .is_some_and(|note| note.contains("the file changed since you read it")),
        "the result must say the file moved: {result}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must exist"),
        b"thread\nthread\nthread\nthread\nthread\n"
    );
}

/// An anchor that occurs nowhere refuses even under `replace_all`: replacing
/// nothing is a miss to re-anchor from, never a silent success.
#[tokio::test]
async fn replace_all_with_no_match_refuses() {
    let sandbox = Sandbox::new("replace-all-miss");
    let seed = b"needle\n";
    sandbox
        .ws
        .write("notes.md", seed)
        .expect("seed write must succeed");
    let error = sandbox
        .tools()
        .execute(
            "edit",
            serde_json::json!({
                "path": "notes.md",
                "old_text": "absent",
                "new_text": "thread",
                "replace_all": true,
                "expected_digest": observed_digest(&sandbox, "notes.md"),
            }),
        )
        .await
        .expect_err("zero matches must refuse under replace_all too");
    assert!(matches!(
        error,
        saya_agent::ToolError::WorkspaceEditNoMatch { matches: 0, .. }
    ));
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.md")).expect("file must survive"),
        seed
    );
}
