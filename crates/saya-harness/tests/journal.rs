//! The run event journal's contract: one append is exactly one NDJSON line,
//! nothing secret-shaped ever reaches the run dir's bytes, and a resume
//! reads back the state the journal recorded.

use std::{fs, path::PathBuf};

#[path = "journal/append.rs"]
mod append;

#[path = "journal/resume.rs"]
mod resume;

#[path = "journal/replay.rs"]
mod replay;

fn temp_run_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "saya-harness-journal-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
