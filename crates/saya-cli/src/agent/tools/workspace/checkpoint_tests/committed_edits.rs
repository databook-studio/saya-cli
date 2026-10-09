use super::*;

#[tokio::test]
async fn replace_commits_one_checkpoint_with_the_exact_before_image() {
    let sandbox = Sandbox::new("replace");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] = "first line\r\ncafé au lait\n".as_bytes();
    fs::write(sandbox.ws_root().join("notes.txt"), original).expect("seed");
    let digest = observed_digest(&sandbox, "notes.txt");

    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.txt",
                "old_text": "café",
                "new_text": "latte",
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("the edit succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let edited = fs::read(sandbox.ws_root().join("notes.txt")).expect("file read");
    assert_ne!(edited, original, "the file was edited");
    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1, "exactly one checkpoint");
    let record = &records[0];
    assert_eq!(record.state, CheckpointState::Committed);
    assert_eq!(record.source, EditSource::WorkspaceEdit);
    assert_eq!(record.path, "notes.txt");
    assert_eq!(
        record.before,
        BeforeImage::Present(saya_types::FileVersion {
            sha256: digest,
            size: original.len() as u64,
        }),
        "the before-image names the original bytes exactly"
    );
    let after = record
        .after
        .as_ref()
        .expect("a committed record carries the post-edit version");
    assert_eq!(
        after.sha256, result["digest"],
        "the after-image names the new bytes"
    );
    assert_eq!(after.size, edited.len() as u64);
    let image = store
        .read_before_image(SESSION, record.id.as_str())
        .expect("the before-image reads back");
    assert_eq!(image, original, "the stored image is the original bytes");
}

#[tokio::test]
async fn append_to_an_existing_file_commits_a_checkpoint() {
    let sandbox = Sandbox::new("append");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] = "head\r\n".as_bytes();
    fs::write(sandbox.ws_root().join("notes.txt"), original).expect("seed");
    let digest = observed_digest(&sandbox, "notes.txt");

    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "notes.txt",
                "offset": original.len(),
                "chunk": "café\n",
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("the append succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let edited = fs::read(sandbox.ws_root().join("notes.txt")).expect("file read");
    assert_eq!(
        edited,
        b"head\r\ncaf\xc3\xa9\n".as_slice(),
        "the file was appended"
    );
    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.state, CheckpointState::Committed);
    assert_eq!(
        record.before,
        BeforeImage::Present(saya_types::FileVersion {
            sha256: digest,
            size: original.len() as u64,
        })
    );
    assert_eq!(
        record.after.as_ref().expect("after-image").sha256,
        result["digest"]
    );
    let image = store
        .read_before_image(SESSION, record.id.as_str())
        .expect("the before-image reads back");
    assert_eq!(image, original);
}

#[tokio::test]
async fn append_create_records_an_absent_before_image() {
    let sandbox = Sandbox::new("append-create");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));

    let result = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "made.txt",
                "offset": 0,
                "chunk": "created\r\n",
            }),
        )
        .await
        .expect("the create succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let written = fs::read(sandbox.ws_root().join("made.txt")).expect("file read");
    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.state, CheckpointState::Committed);
    assert_eq!(record.before, BeforeImage::Absent);
    assert_eq!(
        record.after.as_ref().expect("after-image").sha256,
        result["digest"],
        "the after-image names the created bytes"
    );
    assert_eq!(written, b"created\r\n".as_slice());
}

#[tokio::test]
async fn write_create_records_an_absent_before_image() {
    let sandbox = Sandbox::new("write-create");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));

    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "made.txt",
                "content": "created\r\n",
            }),
        )
        .await
        .expect("the create succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.state, CheckpointState::Committed);
    assert_eq!(record.source, EditSource::WorkspaceWrite);
    assert_eq!(record.before, BeforeImage::Absent);
}

#[tokio::test]
async fn write_guarded_replace_captures_the_exact_before_image() {
    let sandbox = Sandbox::new("write-replace");
    let store = sandbox.sessions_store();
    let tools = sandbox.tools_with_checkpoints(&store, Arc::new(AtomicBool::new(true)));
    let original: &[u8] = "whole file body\n".as_bytes();
    fs::write(sandbox.ws_root().join("notes.txt"), original).expect("seed");
    let digest = observed_digest(&sandbox, "notes.txt");

    let result = tools
        .execute(
            "workspace_write",
            serde_json::json!({
                "path": "notes.txt",
                "content": "replaced body\n",
                "expected_digest": digest.clone(),
            }),
        )
        .await
        .expect("the guarded replace succeeds");
    assert_eq!(result["recovery"], "checkpoint recorded");

    let edited = fs::read(sandbox.ws_root().join("notes.txt")).expect("file read");
    assert_eq!(edited, b"replaced body\n".as_slice());
    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.state, CheckpointState::Committed);
    assert_eq!(
        record.before,
        BeforeImage::Present(saya_types::FileVersion {
            sha256: digest,
            size: original.len() as u64,
        })
    );
    let image = store
        .read_before_image(SESSION, record.id.as_str())
        .expect("the before-image reads back");
    assert_eq!(image, original);
}
