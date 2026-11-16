//! The job recorder's contract: a create that refused left no "absent"
//! before-image behind, and a successful edit records the first bytes while
//! keeping its tool result byte-identical.

use super::*;
use saya_harness::coding::local::before_image::{BeforeImageStore, BeforeState};

/// Tools whose checkpoint recorder is an in-place job's before-image store.
fn job_tools(sandbox: &Sandbox) -> (DatabaseTools, Arc<BeforeImageStore>) {
    let records = sandbox.outer.join("records");
    fs::create_dir_all(&records).expect("job records create");
    let store = Arc::new(BeforeImageStore::open(&records).expect("before-image store opens"));
    let recorder = CheckpointRecorder::job(
        Arc::clone(&store),
        Arc::new(|| 1),
        Arc::new(sandbox.ws.clone()),
    );
    (sandbox.tools().with_checkpoints(Some(recorder)), store)
}

#[tokio::test]
async fn job_recorder_failed_create_leaves_no_absent_entry() {
    let sandbox = Sandbox::new("job-failed-create");
    let (tools, store) = job_tools(&sandbox);
    fs::write(sandbox.ws_root().join("exists.txt"), b"original").expect("seed");

    let error = tools
        .execute(
            "write",
            serde_json::json!({"path": "exists.txt", "content": "replacement"}),
        )
        .await
        .expect_err("a create over an existing file refuses");
    assert!(
        matches!(error, ToolError::WorkspaceDigestRequired { .. }),
        "the refusal is the digest-required precondition: {error:?}"
    );
    assert_eq!(
        fs::read(sandbox.ws_root().join("exists.txt")).expect("file read"),
        b"original",
        "the refused create changed nothing"
    );
    assert!(
        store.entries().expect("entries read").is_empty(),
        "a clean refusal forgets the absent first touch"
    );
}

#[tokio::test]
async fn job_recorder_successful_edit_has_no_recovery_key() {
    let sandbox = Sandbox::new("job-edit");
    let (tools, store) = job_tools(&sandbox);
    let original: &[u8] = b"alpha";
    fs::write(sandbox.ws_root().join("a.txt"), original).expect("seed");
    let digest = observed_digest(&sandbox, "a.txt");

    let result = tools
        .execute(
            "edit",
            serde_json::json!({
                "path": "a.txt",
                "old_text": "alpha",
                "new_text": "beta",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("the edit succeeds");

    assert!(
        result.get("recovery").is_none(),
        "a job's successful result carries no recovery key: {result}"
    );
    let entries = store.entries().expect("entries read");
    assert_eq!(entries.len(), 1, "one first touch");
    let BeforeState::File {
        sha256,
        size,
        executable,
    } = &entries[0].state
    else {
        panic!("expected a recorded file, got {:?}", entries[0].state);
    };
    assert_eq!(sha256.as_str(), saya_types::sha256_hex(original));
    assert_eq!(*size, original.len() as u64);
    assert!(!executable);
    assert_eq!(
        store.read_image(&entries[0]).expect("image reads"),
        Some(original.to_vec()),
        "the first bytes are kept"
    );
}
