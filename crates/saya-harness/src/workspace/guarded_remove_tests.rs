//! Adversarial tests for the guarded removal: the digest guard, the entry
//! kinds it refuses, the containment refusals, and the race contract among
//! Saya's own serialized writers. Mirrors the guarded whole-file write's
//! tests in `tests/workspace_patch.rs`.

#[path = "guarded_remove_tests/concurrency.rs"]
mod concurrency;
#[path = "guarded_remove_tests/digest_guard.rs"]
mod digest_guard;
#[path = "guarded_remove_tests/happy_path.rs"]
mod happy_path;
#[path = "guarded_remove_tests/invalid_targets.rs"]
mod invalid_targets;
#[path = "guarded_remove_tests/non_file_targets.rs"]
mod non_file_targets;
#[path = "guarded_remove_tests/symlink_refusals.rs"]
mod symlink_refusals;

use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::workspace::Workspace;

/// SHA-256 of the empty byte string — a digest no non-empty fixture hashes to.
const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer = std::env::temp_dir().join(format!("saya-rm-{label}-{}", std::process::id()));
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

fn bytes_of(ws: &Workspace, rel: &str) -> Vec<u8> {
    fs::read(ws.root().join(rel)).unwrap()
}

fn stray_names(sandbox: &Sandbox) -> Vec<String> {
    fs::read_dir(sandbox.root())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(".saya-tmp-"))
        .collect()
}
