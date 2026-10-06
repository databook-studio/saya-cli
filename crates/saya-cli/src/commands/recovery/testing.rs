//! Test scaffolding for the recovery operations: a sandbox workspace with
//! a sibling session store, and helpers that record edits through the
//! real `FsSessionStore` lifecycle (prepare → commit), matching the
//! capture path the file tools use.

use std::fs;
use std::path::PathBuf;

use saya_harness::workspace::Workspace;
use saya_store::{FsSessionStore, NewCheckpoint};
use saya_types::{CheckpointPolicy, EditCheckpoint, EditSource, FileVersion};

use super::sha256_hex;

/// The one session id every test uses.
pub(crate) const SESSION: &str = "sess-1";

/// A workspace under the OS temp dir, removed on drop. The workspace root
/// sits at `<outer>/ws`; the session store at `<outer>/sessions`, so the
/// private checkpoint data lands outside the project.
pub(crate) struct Sandbox {
    pub(crate) outer: PathBuf,
    pub(crate) ws: Workspace,
    pub(crate) store: FsSessionStore,
}

impl Sandbox {
    pub(crate) fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-recovery-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        let store = FsSessionStore::new(outer.join("sessions"));
        Self { outer, ws, store }
    }

    pub(crate) fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    pub(crate) fn checkpoints_dir(&self) -> PathBuf {
        self.outer
            .join("sessions")
            .join(SESSION)
            .join("checkpoints")
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// Records one committed edit through the real store lifecycle: prepare
/// with the exact before bytes when the file existed (`None` records a
/// create), then commit with `after` as the post-edit version.
pub(crate) fn record_commit(
    sandbox: &Sandbox,
    path: &str,
    source: EditSource,
    before: Option<&[u8]>,
    after: &[u8],
    now_unix_ms: i64,
) -> EditCheckpoint {
    let policy = CheckpointPolicy { enabled: true };
    let record = sandbox
        .store
        .prepare_checkpoint(
            SESSION,
            &policy,
            NewCheckpoint {
                path,
                source,
                before,
            },
            now_unix_ms,
        )
        .expect("the checkpoint must prepare");
    sandbox
        .store
        .commit_checkpoint(
            SESSION,
            record.id.as_str(),
            FileVersion {
                sha256: sha256_hex(after),
                size: after.len() as u64,
            },
            now_unix_ms,
        )
        .expect("the checkpoint must commit");
    record
}

/// Records a `Prepared` checkpoint: the before-image is stored but the
/// edit is not known to have landed.
pub(crate) fn record_prepared(
    sandbox: &Sandbox,
    path: &str,
    before: Option<&[u8]>,
    now_unix_ms: i64,
) -> EditCheckpoint {
    let policy = CheckpointPolicy { enabled: true };
    sandbox
        .store
        .prepare_checkpoint(
            SESSION,
            &policy,
            NewCheckpoint {
                path,
                source: EditSource::WorkspaceEdit,
                before,
            },
            now_unix_ms,
        )
        .expect("the checkpoint must prepare")
}

/// The digest a committed record's after-image carries for `bytes`.
pub(crate) fn digest(bytes: &[u8]) -> String {
    sha256_hex(bytes)
}
