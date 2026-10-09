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

use super::workspace::checkpoint::{
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

#[path = "checkpoint_tests/committed_edits.rs"]
mod committed_edits;
#[path = "checkpoint_tests/recovery_classification.rs"]
mod recovery_classification;
#[path = "checkpoint_tests/refusals.rs"]
mod refusals;
