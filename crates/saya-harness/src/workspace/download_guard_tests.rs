//! Adversarial tests for the download promotion's destination guard: the
//! create-only default, the digest-replace precondition, and the refusals
//! that leave the destination and the completed part untouched. Real temp
//! workspaces and real files throughout — the guard is exercised through
//! `Workspace::promote_download`, the dispatcher the download machinery
//! calls, never through a mock.

#[path = "download_guard_tests/create_only.rs"]
mod create_only;
#[path = "download_guard_tests/digest_replace.rs"]
mod digest_replace;

use std::fs;

use crate::workspace::Workspace;

const DEST: &str = "downloads/doc.bin";
const PART: &str = "downloads/doc.bin.saya-part";

/// SHA-256 of the empty byte string — a digest no non-empty fixture hashes
/// to.
const EMPTY_DIGEST: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

struct Sandbox {
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("saya-dl-guard-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let ws = Workspace::open(&root).expect("workspace root must open");
        Self { ws }
    }

    fn root(&self) -> &std::path::Path {
        self.ws.root()
    }

    /// Seeds the completed part with the content the promotion must land at
    /// the destination.
    fn seed_part(&self, bytes: &[u8]) {
        self.ws.write(PART, bytes).unwrap();
    }

    fn part_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.root().join(PART)).ok()
    }

    fn dest_bytes(&self) -> Option<Vec<u8>> {
        fs::read(self.root().join(DEST)).ok()
    }

    /// The destination's whole-content digest as a caller would have
    /// observed it through a contained read.
    fn observed_dest_digest(&self) -> String {
        self.ws.read(DEST, 4096).unwrap().digest
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(self.root());
    }
}
