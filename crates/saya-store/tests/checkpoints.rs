//! Private checkpoint before-image storage (saya-store): the public-API
//! contract for opt-in recovery of direct file-tool edits. Checkpoints are
//! private recovery data — exact before-image bytes, never redacted — kept
//! under `<sessions>/<id>/checkpoints/` so the session's deletion removes
//! them. The gates asserted here: policy opt-in writes nothing when
//! disabled, quota admission refuses before any write, a failed prepare
//! leaves no half state, reads verify digest and size, crash recovery
//! converts `Prepared` to `Uncertain` and removes only orphan images and
//! staging temps, commit is idempotent on the same after-image, and
//! tombstoned sessions and symlinked paths are refused. Admission
//! screening refuses credential-file paths, known-secret content, and
//! database-export artifacts before anything is written, and ordinary
//! source still checkpoints normally.

use saya_store::{
    CheckpointStoreError, DeleteStatus, FsSessionStore, NewCheckpoint, RecoveryReport,
    RedactedSession, ScreenRefusal, SessionStore, StoreError,
};
use saya_types::{
    BeforeImage, CheckpointPolicy, CheckpointRefusal, CheckpointState, CheckpointUsage,
    EditCheckpoint, EditSource, FileVersion, MAX_CHECKPOINT_FILE_BYTES,
    MAX_CHECKPOINT_SESSION_BYTES,
};
use sha2::{Digest, Sha256};
use std::{collections::HashSet, fs, path::Path, path::PathBuf};

const SESSION: &str = "worker";

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "saya-store-checkpoints-{label}-{}",
        std::process::id()
    ))
}

/// Builds the store at `<tmp>/sessions` so `<tmp>` is the state dir whose
/// independent contents must survive a session deletion.
fn ck_store(label: &str) -> (PathBuf, FsSessionStore) {
    let state = temp_root(label);
    let _ = fs::remove_dir_all(&state);
    (state.clone(), FsSessionStore::new(state.join("sessions")))
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(future)
}

fn ck_dir(state: &Path, session: &str) -> PathBuf {
    state.join("sessions").join(session).join("checkpoints")
}

fn entry_count(dir: &Path) -> usize {
    fs::read_dir(dir).unwrap().count()
}

fn policy() -> CheckpointPolicy {
    CheckpointPolicy { enabled: true }
}

fn digest_of(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(bytes);
    hash.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn after_of(bytes: &[u8]) -> FileVersion {
    FileVersion {
        sha256: digest_of(bytes),
        size: bytes.len() as u64,
    }
}

fn plant_session(store: &FsSessionStore, id: &str) {
    block_on(store.save(RedactedSession {
        id: id.to_owned(),
        ..Default::default()
    }))
    .unwrap();
}

fn prepare(
    store: &FsSessionStore,
    session: &str,
    path: &str,
    before: Option<&[u8]>,
    now: i64,
) -> Result<EditCheckpoint, CheckpointStoreError> {
    store.prepare_checkpoint(
        session,
        &policy(),
        NewCheckpoint {
            path,
            source: EditSource::WorkspaceEdit,
            before,
        },
        now,
    )
}

#[path = "checkpoints/order_and_delete.rs"]
mod order_and_delete;
#[path = "checkpoints/preparation.rs"]
mod preparation;
#[path = "checkpoints/recovery.rs"]
mod recovery;
#[path = "checkpoints/screening.rs"]
mod screening;
#[path = "checkpoints/session_safety.rs"]
mod session_safety;
#[path = "checkpoints/transitions.rs"]
mod transitions;
