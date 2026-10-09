//! Tests for the contained search surface: `glob` and `grep` over the
//! contained walk. The defining invariants: candidates are only ever paths
//! the containment walk itself resolved (never a symlink, never outside),
//! bounds fail as typed errors rather than truncating quietly, and grep
//! serves honest coverage (`files_skipped`) instead of mojibake or a prefix
//! scan that reads as full coverage.

use std::{fs, path::PathBuf};

use saya_harness::workspace::Workspace;

#[path = "workspace_search/glob.rs"]
mod glob;

#[path = "workspace_search/grep.rs"]
mod grep;

#[path = "workspace_search/bound_backstop.rs"]
mod bound_backstop;

#[path = "workspace_search/symlink.rs"]
mod symlink;

/// Keeps a sandbox directory alive for the test and removes it afterwards.
/// The workspace root sits one level down (`outer/ws`) so outside symlink
/// targets have real places to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wssearch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn plant_tree(ws: &saya_harness::workspace::Workspace) {
    for path in ["a.md", "notes/b.md", "notes/deep/c.md", "code/x.rs"] {
        ws.write(path, b"content")
            .expect("contained write must succeed");
    }
}
