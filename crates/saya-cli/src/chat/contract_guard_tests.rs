//! The chat layer carries no presentation: a front decides how an event looks,
//! so no file here may name the terminal crates the TUI draws with or this
//! crate's renderer.
//!
//! This guard is the one file exempt from its own rule — it has to spell the
//! names it refuses.

use std::path::{Path, PathBuf};

/// What no chat file may name: the two terminal crates, and the renderer whose
/// formatting and colour are the front's business alone.
const FORBIDDEN: &[&str] = &["ratatui", "crossterm", "render::"];

/// The one file allowed to spell them.
const GUARD: &str = "contract_guard_tests.rs";

#[test]
fn no_chat_file_names_a_presentation_layer() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/chat");
    let mut files = Vec::new();
    collect(&root, &mut files);
    assert!(
        files.len() > 1,
        "the chat tree holds only the guard: nothing was scanned"
    );

    let mut offenders = Vec::new();
    for file in files {
        let Some(name) = file.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let relative = file
            .strip_prefix(&root)
            .expect("a file below src/chat")
            .to_string_lossy()
            .replace('\\', "/");
        if name == GUARD {
            continue;
        }
        let source = std::fs::read_to_string(&file).expect("a readable chat source file");
        for (index, line) in source.lines().enumerate() {
            if let Some(forbidden) = FORBIDDEN.iter().find(|word| line.contains(*word)) {
                offenders.push(format!("{relative}:{}: {forbidden}", index + 1));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "presentation in the presentation-free chat layer:\n{}",
        offenders.join("\n")
    );
}

/// Every file in the tree, not only the Rust ones: a chat layer is
/// presentation-free whatever it is written in.
fn collect(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).expect("a readable directory") {
        let path = entry.expect("a readable entry").path();
        if path.is_dir() {
            collect(&path, files);
        } else {
            files.push(path);
        }
    }
}
