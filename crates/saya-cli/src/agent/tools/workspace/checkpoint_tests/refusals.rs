use super::*;

#[tokio::test]
async fn a_write_with_a_stale_digest_refuses_and_leaves_no_checkpoint() {
    let sandbox = Sandbox::new("write-stale");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] = "whole file body\n".as_bytes();
    fs::write(sandbox.ws_root().join("notes.txt"), original).expect("seed");

    let error = tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "notes.txt",
                "content": "replaced body\n",
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("a stale digest refuses the write");
    assert!(
        matches!(&error, ToolError::WorkspaceWrite(text) if text.contains("changed since")),
        "expected the file-changed refusal, got: {error}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("file read"),
        original,
        "the refused write leaves the file byte-identical"
    );
    assert!(
        store.list_checkpoints(SESSION).expect("list").is_empty(),
        "no checkpoint is left behind by the refused edit"
    );
}

#[tokio::test]
async fn a_disabled_recorder_matches_the_no_recorder_result_and_stores_nothing() {
    let recorded = Sandbox::new("off-recorded");
    let plain = Sandbox::new("off-plain");
    let store = recorded.sessions_store();
    for sandbox in [&recorded, &plain] {
        fs::write(
            sandbox.ws_root().join("notes.txt"),
            b"alpha beta\n".as_slice(),
        )
        .expect("seed");
    }
    let args = |sandbox: &Sandbox| {
        serde_json::json!({
            "path": "notes.txt",
            "old_text": "beta",
            "new_text": "BETA",
            "expected_digest": observed_digest(sandbox, "notes.txt"),
        })
    };
    let recorded_args = args(&recorded);
    let plain_args = args(&plain);

    let recorded_tools = recorded.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(false)));
    let recorded_result = recorded_tools
        .execute("workspace_edit", recorded_args)
        .await
        .expect("the edit succeeds");
    let plain_result = plain
        .tools()
        .execute("workspace_edit", plain_args)
        .await
        .expect("the edit succeeds");

    assert_eq!(
        recorded_result, plain_result,
        "a disabled recorder changes nothing in the result"
    );
    assert!(recorded_result.get("recovery").is_none());
    assert!(store.list_checkpoints(SESSION).expect("list").is_empty());
    assert!(
        !recorded
            .outer
            .join("sessions")
            .join(SESSION)
            .join("checkpoints")
            .exists(),
        "no checkpoints directory is created"
    );
}

#[tokio::test]
async fn a_capture_refusal_prevents_the_edit_and_leaves_the_file_unchanged() {
    let sandbox = Sandbox::new("quota");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] = "alpha beta\n".as_bytes();
    fs::write(sandbox.ws_root().join("notes.txt"), original).expect("seed");
    let policy = CheckpointPolicy { enabled: true };
    for _ in 0..MAX_CHECKPOINTS_PER_SESSION {
        store
            .prepare_checkpoint(
                SESSION,
                &policy,
                NewCheckpoint {
                    path: "pad.txt",
                    source: EditSource::WorkspaceEdit,
                    before: Some(b"pad".as_slice()),
                },
                1_700_000_000_000,
            )
            .expect("the quota admits up to the bound");
    }

    let digest = observed_digest(&sandbox, "notes.txt");
    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.txt",
                "old_text": "beta",
                "new_text": "BETA",
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("a capture refusal prevents the edit");
    let ToolError::WorkspaceEdit(text) = &error else {
        panic!("expected the edit tool's refusal, got: {error}");
    };
    assert!(
        text.contains("cannot be checkpointed"),
        "the refusal names the quota: {text}"
    );
    assert!(
        text.contains("too many checkpoints"),
        "the refusal names the quota class: {text}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("file read"),
        original,
        "the refused capture leaves the file byte-identical"
    );
    assert_eq!(
        store.list_checkpoints(SESSION).expect("list").len() as u32,
        MAX_CHECKPOINTS_PER_SESSION,
        "no new checkpoint is added"
    );
}

#[tokio::test]
async fn screened_files_are_edited_without_a_checkpoint() {
    // A credential path: the edit proceeds, honestly unrecorded.
    let path_sandbox = Sandbox::new("screen-path");
    let path_store = path_sandbox.sessions_store();
    let path_tools =
        path_sandbox.tools_with_checkpoints(&path_store, Arc::new(AtomicBool::new(true)));
    fs::write(path_sandbox.ws_root().join(".env"), b"A=1\n").expect("seed");
    let digest = observed_digest(&path_sandbox, ".env");
    let result = path_tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": ".env",
                "old_text": "A",
                "new_text": "B",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the screened edit proceeds");
    assert_eq!(result["recovery"], "not recorded: credential path");
    assert!(
        path_store
            .list_checkpoints(SESSION)
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        fs::read(path_sandbox.ws_root().join(".env")).expect("file read"),
        b"B=1\n".as_slice(),
        "the screened file was edited"
    );

    // Secret-shaped content in an ordinary file: same honest refusal.
    let content_sandbox = Sandbox::new("screen-content");
    let content_store = content_sandbox.sessions_store();
    let content_tools =
        content_sandbox.tools_with_checkpoints(&content_store, Arc::new(AtomicBool::new(true)));
    fs::write(
        content_sandbox.ws_root().join("notes.txt"),
        b"API_KEY=abc\n",
    )
    .expect("seed");
    let digest = observed_digest(&content_sandbox, "notes.txt");
    let result = content_tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.txt",
                "old_text": "abc",
                "new_text": "def",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the screened edit proceeds");
    assert_eq!(result["recovery"], "not recorded: secret-shaped content");
    assert!(
        content_store
            .list_checkpoints(SESSION)
            .expect("list")
            .is_empty()
    );
    assert_eq!(
        fs::read(content_sandbox.ws_root().join("notes.txt")).expect("file read"),
        b"API_KEY=def\n".as_slice(),
        "the screened file was edited"
    );
}

#[tokio::test]
async fn a_clean_refusal_leaves_no_checkpoint() {
    let sandbox = Sandbox::new("clean-refusal");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] = "alpha beta\n".as_bytes();
    fs::write(sandbox.ws_root().join("notes.txt"), original).expect("seed");

    let error = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.txt",
                "old_text": "beta",
                "new_text": "BETA",
                "expected_digest": "0".repeat(64),
            }),
        )
        .await
        .expect_err("a stale anchor refuses the replace");
    assert!(
        matches!(error, ToolError::WorkspaceEditMoved { .. }),
        "expected the typed moved-anchor refusal"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("notes.txt")).expect("file read"),
        original,
        "the refused edit leaves the file byte-identical"
    );
    assert!(store.list_checkpoints(SESSION).expect("list").is_empty());
}
