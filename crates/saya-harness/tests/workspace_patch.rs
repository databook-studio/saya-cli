//! S1: `Workspace::patch_range` — anchored range-replace, atomically committed.
//!
//! The primitive takes a resolved target, a byte range, a positional
//! precondition (the size the caller measured when choosing the range), and
//! the replacement bytes. Anchor-finding (matching text, counting matches)
//! is a later slice's job — this operation only splices the range it is
//! given, under the same containment and atomic commit as the whole-file
//! write.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use saya_harness::HarnessError;
use saya_harness::workspace::MAX_IO_BYTES;
use saya_harness::workspace::Workspace;
use saya_harness::workspace::patch::{PATCH_MAX_FILE_BYTES, PATCH_REPLACEMENT_MAX_BYTES};

const INSIDE: &[u8] = b"inside-sentinel";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer = std::env::temp_dir().join(format!("saya-patch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).unwrap();
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn root(&self) -> &Path {
        self.ws.root()
    }

    fn outside_path(&self, name: &str) -> PathBuf {
        self.outer.join(name)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn is_rejection(error: &HarnessError) -> bool {
    matches!(
        error,
        HarnessError::PathOutsideRoot { .. }
            | HarnessError::InvalidPath { .. }
            | HarnessError::SymlinkRefused { .. }
            | HarnessError::DeniedName { .. }
            | HarnessError::BoundsExceeded { .. }
            | HarnessError::AlreadyExists { .. }
            | HarnessError::IdentityChanged { .. }
            | HarnessError::NotRegularFile { .. }
            | HarnessError::RangeOutOfBounds { .. }
            | HarnessError::LengthMismatch { .. }
            | HarnessError::ContentChanged { .. }
            | HarnessError::NotFound { .. }
            | HarnessError::Io { .. }
    )
}

fn bytes_of(ws: &Workspace, rel: &str) -> Vec<u8> {
    fs::read(ws.root().join(rel)).unwrap()
}

#[path = "workspace_patch/patch_refusals.rs"]
mod patch_refusals;

#[path = "workspace_patch/range_commit.rs"]
mod range_commit;

#[path = "workspace_patch/containment_races.rs"]
mod containment_races;

#[path = "workspace_patch/platform_identity.rs"]
mod platform_identity;

/// The containment corpus is generated, not enumerated: traversal, absolute,
/// drive/UNC, empty, dot, hygiene, and symlink-planted shapes are joined
/// into mixed forms, and no generated argument may ever alter the outside
/// sentinel or plant a name outside the root.
#[path = "workspace_patch/property.rs"]
mod property;

#[path = "workspace_patch/create_new.rs"]
mod create_new;

#[path = "workspace_patch/guarded_write.rs"]
mod guarded_write;
