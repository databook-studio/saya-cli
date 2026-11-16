//! The bounded tail of a job's diagnostic and log files, so a failure that
//! only reproduces on real Windows names its own cause instead of a bare
//! `mandatory_persistence`. Paths, counts and bytes are bounded; nothing here
//! carries authority.

use std::{
    io::{Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

const TAIL_BYTES: u64 = 4 * 1024;
const MAX_FILES: usize = 8;
const MAX_VISITED: usize = 128;

fn diagnostic_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default();
    name.ends_with(".jsonl")
        || name.ends_with(".log")
        || name == "diagnostics-summary.json"
        || name == "diagnostics-summary.stage"
}

fn collect(directory: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 6 || found.len() >= MAX_VISITED {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect(&path, depth + 1, found);
        } else if kind.is_file() && diagnostic_file(&path) {
            found.push(path);
        }
    }
}

fn tail(path: &Path) -> String {
    let Ok(mut file) = std::fs::File::open(path) else {
        return "<unreadable>".to_owned();
    };
    let len = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
    let start = len.saturating_sub(TAIL_BYTES);
    if file.seek(SeekFrom::Start(start)).is_err() {
        return "<unseekable>".to_owned();
    }
    let mut bytes = Vec::new();
    if file.take(TAIL_BYTES).read_to_end(&mut bytes).is_err() {
        return "<unreadable>".to_owned();
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

pub(super) fn dump(root: &Path) -> String {
    let mut found = Vec::new();
    collect(root, 0, &mut found);
    found.sort_by_key(|path| {
        std::cmp::Reverse(
            path.metadata()
                .and_then(|metadata| metadata.modified())
                .ok(),
        )
    });
    found.truncate(MAX_FILES);
    if found.is_empty() {
        return format!("no diagnostic or log files under {root:?}");
    }
    found
        .into_iter()
        .map(|path| format!("--- {} ---\n{}", path.display(), tail(&path)))
        .collect::<Vec<_>>()
        .join("\n")
}
