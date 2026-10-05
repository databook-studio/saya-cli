//! Tests for checkpoint capture around the DIRECT FILE-TOOL EDITS: with a
//! recorder attached and recovery enabled, every successful direct edit
//! carries an exact committed checkpoint (or is honestly declared
//! unrecorded), a refused edit leaves nothing behind, and a capture
//! refusal prevents the edit from running. Without a recorder — or with
//! the enable flag off — results are byte-identical to the unrecorded run
//! and no recovery data is stored. The recorder is attached here directly;
//! the session wiring is the next packet.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::*;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::workspace::Workspace;
use saya_store::{FsSessionStore, NewCheckpoint, ScreenRefusal};
use saya_types::{
    BeforeImage, CheckpointPolicy, CheckpointRefusal, CheckpointState, EditSource,
    MAX_CHECKPOINTS_PER_SESSION,
};

use super::database_tools::checkpoint::{
    CheckpointRecorder, FILE_CHANGED_SINCE_OBSERVED, FILE_EXISTS_REQUIRES_DIGEST,
    MISSING_FILE_NOT_A_REPLACE, RefusalClass, classify_edit, classify_write, quota_refusal_text,
    screen_class_text,
};

/// A sandbox workspace under the OS temp dir, removed on drop. The root
/// sits one level down (`outer/ws`) so a `..` escape has a real target to
/// name, and a sibling `outer/sessions` holds the checkpoint store.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsckpt-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    fn sessions_store(&self) -> Arc<FsSessionStore> {
        Arc::new(FsSessionStore::new(self.outer.join("sessions")))
    }

    /// Database tools with the sandbox workspace attached and no
    /// connections — a workspace-only run has no selected profile.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }

    /// Tools with a checkpoint recorder for `sess-1` over the given flag.
    fn tools_with_checkpoints(
        &self,
        store: &Arc<FsSessionStore>,
        enabled: Arc<AtomicBool>,
    ) -> DatabaseTools {
        self.tools().with_checkpoints(Some(CheckpointRecorder::new(
            store.clone(),
            "sess-1".into(),
            enabled,
        )))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// The digest the model observes from a fresh read: the replace variant
/// and the guarded write require it as `expected_digest`.
fn observed_digest(sandbox: &Sandbox, path: &str) -> String {
    sandbox
        .ws
        .read(path, 0)
        .expect("the seeded file must read")
        .digest
}

/// The one session id every test uses.
const SESSION: &str = "sess-1";

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

#[tokio::test]
async fn a_crash_after_the_commit_leaves_an_uncertain_record_on_recovery() {
    let sandbox = Sandbox::new("crash");
    let store = sandbox.sessions_store();
    let recorder = CheckpointRecorder::new(
        store.clone(),
        SESSION.into(),
        Arc::new(AtomicBool::new(true)),
    );

    // A capture prepared, the file commit landed, then the process died
    // before `finish_ok`: the record stays `Prepared` until recovery runs.
    let capture = recorder.begin(
        "notes.txt",
        EditSource::WorkspaceEdit,
        Some(b"before".as_slice()),
    );
    drop(capture);

    let report = store.recover_checkpoints(SESSION).expect("recovery runs");
    assert_eq!(report.marked_uncertain, 1);
    let records = store.list_checkpoints(SESSION).expect("list");
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, CheckpointState::Uncertain);
}

#[test]
fn edit_refusals_classify_clean_only_when_nothing_was_written() {
    let clean = [
        ToolError::WorkspaceEditMoved {
            path: "p".into(),
            expected_size: None,
            current_size: 1,
            expected_digest: None,
            current_digest: "d".into(),
        },
        ToolError::WorkspaceAppendOffset {
            path: "p".into(),
            expected_offset: 0,
            current_size: 1,
            current_digest: "d".into(),
        },
    ];
    for error in &clean {
        assert_eq!(classify_edit(error), RefusalClass::Clean);
    }
    let ambiguous = [
        ToolError::WorkspaceEdit("harness failure: identity changed after a rename".into()),
        ToolError::WorkspaceEditNotText { path: "p".into() },
        ToolError::WorkspaceWrite("unexpected channel".into()),
    ];
    for error in &ambiguous {
        assert_eq!(classify_edit(error), RefusalClass::Ambiguous);
    }
}

#[test]
fn write_refusals_classify_clean_only_for_the_constant_preconditions() {
    let clean = [
        ToolError::WorkspaceWrite(FILE_EXISTS_REQUIRES_DIGEST.to_owned()),
        ToolError::WorkspaceWrite(FILE_CHANGED_SINCE_OBSERVED.to_owned()),
        ToolError::WorkspaceWrite(MISSING_FILE_NOT_A_REPLACE.to_owned()),
    ];
    for error in &clean {
        assert_eq!(classify_write(error), RefusalClass::Clean);
    }
    let ambiguous = [
        ToolError::WorkspaceWrite("identity changed after a rename".into()),
        ToolError::WorkspaceEditMoved {
            path: "p".into(),
            expected_size: None,
            current_size: 1,
            expected_digest: None,
            current_digest: "d".into(),
        },
        ToolError::WorkspaceEdit("unexpected channel".into()),
    ];
    for error in &ambiguous {
        assert_eq!(classify_write(error), RefusalClass::Ambiguous);
    }
}

#[test]
fn quota_and_screen_refusals_map_to_model_safe_texts() {
    assert_eq!(
        quota_refusal_text(CheckpointRefusal::FileTooLarge),
        "recovery is enabled but this edit cannot be checkpointed (file too large); \
         the file was not changed"
    );
    assert_eq!(
        quota_refusal_text(CheckpointRefusal::TooManyCheckpoints),
        "recovery is enabled but this edit cannot be checkpointed (too many checkpoints); \
         the file was not changed"
    );
    assert_eq!(
        quota_refusal_text(CheckpointRefusal::SessionBytesExceeded),
        "recovery is enabled but this edit cannot be checkpointed \
         (session recovery storage is full); the file was not changed"
    );
    assert_eq!(
        screen_class_text(ScreenRefusal::CredentialPath),
        "credential path"
    );
    assert_eq!(
        screen_class_text(ScreenRefusal::SecretContent),
        "secret-shaped content"
    );
    assert_eq!(
        screen_class_text(ScreenRefusal::DatabaseExport),
        "database export"
    );
}
