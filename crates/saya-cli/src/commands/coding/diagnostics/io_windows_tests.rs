//! Windows-only behavioural tests of the diagnostic file handles: real
//! Windows refuses `FlushFileBuffers` and `SetEndOfFile` on an append-only
//! handle (`FILE_APPEND_DATA` without `FILE_WRITE_DATA`).

use super::*;
use std::{
    io::{Seek as _, SeekFrom},
    path::{Path, PathBuf},
};

fn root() -> PathBuf {
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path =
        std::env::temp_dir().join(format!("saya-diagnostic-io-{}-{stamp}", std::process::id()));
    std::fs::create_dir(&path).unwrap();
    path
}

/// Real Windows refuses `SetEndOfFile` on an append-only handle
/// (`FILE_APPEND_DATA` without `FILE_WRITE_DATA`), which is the
/// `diagnostic_storage` fault this models. An append handle also ignores
/// the write position, so a write at offset zero proves the rewrite handle
/// is not append-mode without depending on an access-mask query.
#[test]
fn rewriting_uses_a_handle_that_writes_at_the_position_and_truncates() {
    let root = root();
    let path: &Path = &root.join("diagnostics-summary.json");
    std::fs::write(path, b"old").unwrap();
    let mut file = open_rewrite(&root, "diagnostics-summary.json").unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"X").unwrap();
    file.set_len(2).unwrap();
    file.write_all(b"Y").unwrap();
    drop(file);
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"XY",
        "the rewrite handle writes at the position instead of appending"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A durable record append must flush and land at the end; the handle must
/// also accept a positional write, proving `FILE_WRITE_DATA` is present.
#[test]
fn record_appends_flush_and_write_at_the_seek_position() {
    let root = root();
    let name = "tool-events.jsonl";
    let mut file = open(&root, name, true).unwrap();
    file.write_all(b"first\n").unwrap();
    file.sync_all().unwrap();
    file.seek(SeekFrom::End(0)).unwrap();
    file.write_all(b"second\n").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(std::fs::read(root.join(name)).unwrap(), b"first\nsecond\n");
    let mut file = open(&root, name, true).unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();
    file.write_all(b"X").unwrap();
    file.sync_all().unwrap();
    drop(file);
    assert_eq!(
        std::fs::read(root.join(name)).unwrap(),
        b"Xirst\nsecond\n",
        "the record handle writes at the position instead of appending"
    );
    let _ = std::fs::remove_dir_all(&root);
}
