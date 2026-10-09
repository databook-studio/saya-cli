//! Tests for the `workspace_edit` tool's `append` variant: offset-checked
//! chunked writes over the same atomic commit the `replace` variant uses.
//! Append is a range replace — an empty range at EOF with a positional
//! precondition — so every refusal here leaves the file byte-identical, and
//! every mismatch reports the current size the model resumes from.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::ToolExecutor;
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_EDIT_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `..` escape has a real target to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsappend-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    /// Database tools with the sandbox workspace attached and no connections —
    /// a workspace-only run has no selected profile.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

/// The digest the model observes from a fresh read: appending to an
/// existing file requires it as `expected_digest`, so every append to a
/// seeded file names it from a real harness read of the current bytes.
fn observed_digest(sandbox: &Sandbox, path: &str) -> String {
    sandbox
        .ws
        .read(path, 0)
        .expect("the seeded file must read")
        .digest
}

#[path = "workspace_edit_append_tests/digest_and_atomicity.rs"]
mod digest_and_atomicity;
#[path = "workspace_edit_append_tests/guarded_refusals.rs"]
mod guarded_refusals;
#[path = "workspace_edit_append_tests/offset_and_chunking.rs"]
mod offset_and_chunking;
#[path = "workspace_edit_append_tests/payload_arguments.rs"]
mod payload_arguments;
