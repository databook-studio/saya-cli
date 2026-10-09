//! The session journal's properties: one line per event in the settled
//! shape, redacted at write through the repo's own redaction seam, append
//! only, 0600, torn-tail tolerant, corrupt-line fail-closed.

#[path = "session_journal_tests/appends.rs"]
mod appends;
#[path = "session_journal_tests/bounds.rs"]
mod bounds;
#[path = "session_journal_tests/reads.rs"]
mod reads;
#[path = "session_journal_tests/shape.rs"]
mod shape;

use std::path::PathBuf;

/// A fresh state directory per test, the way a session's is created (0700).
fn state_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("saya-journal-{label}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("state dir creates");
    dir
}
