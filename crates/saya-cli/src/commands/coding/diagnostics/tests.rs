use super::*;
use std::{io::Write, path::Path};

pub(super) struct Root(std::path::PathBuf);
impl Root {
    pub(super) fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
pub(super) fn root() -> Root {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!(
        "saya-diagnostic-{}-{timestamp}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let mut builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&path).unwrap();
    Root(path)
}
pub(super) fn record() -> Vec<u8> {
    // JSON whitespace exercises the real byte limit without inventing payload.
    let mut bytes = br#"{"tool":"helpers","status":"completed"}"#.to_vec();
    bytes.resize(7 * 1024, b' ');
    bytes
}
fn write(root: &Path, name: &str, bytes: &[u8]) {
    io::open(root, name, true)
        .unwrap()
        .write_all(bytes)
        .unwrap();
}
#[test]
fn every_stream_rotates_with_exact_bounded_retained_history() {
    let root = root();
    let record = record();
    for stream in Stream::ALL {
        for _ in 0..460 {
            append(root.path(), stream, &record).unwrap();
        }
    }
    let history = history(root.path()).unwrap();
    assert_eq!(history.state, HistoryState::Truncated);
    for entry in &history.streams {
        assert!(entry.current.bytes <= SEGMENT_BYTES as u64);
        assert!(entry.previous.as_ref().unwrap().bytes <= SEGMENT_BYTES as u64);
        assert!(entry.generation.unwrap() >= 3);
        assert_eq!(
            entry.evicted_records.unwrap()
                + entry.current.records
                + entry.previous.as_ref().unwrap().records,
            460
        );
        assert_eq!(
            entry.evicted_bytes.unwrap()
                + entry.current.bytes
                + entry.previous.as_ref().unwrap().bytes,
            460 * (record.len() as u64 + 1)
        );
    }
    let files = std::fs::read_dir(root.path())
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(files.len(), 8); // six segments, one summary, one stable lock
    assert!(
        files
            .iter()
            .map(|entry| entry.metadata().unwrap().len())
            .sum::<u64>()
            <= (6 * SEGMENT_BYTES + SUMMARY_BYTES) as u64
    );
    assert!(!root.path().join(STAGE).exists());
    let exported = export(root.path()).unwrap();
    assert_eq!(exported["history"]["state"], "truncated");
    assert_eq!(exported["files"].as_array().unwrap().len(), 6);
}
#[test]
fn interrupted_tail_rotation_and_summary_recover_as_unknown() {
    for point in ["tail", "before_rename", "after_rename", "summary"] {
        let root = root();
        append(root.path(), Stream::ToolEvents, br#"{"tool":"helpers"}"#).unwrap();
        io::mark_pending(root.path()).unwrap();
        match point {
            "tail" => write(root.path(), Stream::ToolEvents.name(), b"{\"partial\":"),
            "before_rename" => {
                io::remove(root.path(), Stream::ToolEvents.previous()).unwrap();
            }
            "after_rename" => {
                io::rename(
                    root.path(),
                    Stream::ToolEvents.name(),
                    Stream::ToolEvents.previous(),
                )
                .unwrap();
            }
            "summary" => {
                io::truncate(root.path(), SUMMARY, 0).unwrap();
                write(root.path(), SUMMARY, b"{");
            }
            _ => unreachable!(),
        }
        assert_eq!(
            history(root.path()).unwrap().state,
            HistoryState::Unknown,
            "{point}"
        );
        append(
            root.path(),
            Stream::ToolEvents,
            br#"{"tool":"helpers","status":"completed"}"#,
        )
        .unwrap();
        let history = history(root.path()).unwrap();
        assert_eq!(history.state, HistoryState::Unknown);
        assert_eq!(history.streams[0].evicted_records, None);
        assert_eq!(history.streams[0].generation, None);
        assert!(!root.path().join(STAGE).exists());
        let exported = export(root.path()).unwrap();
        assert!(!exported.to_string().contains("partial"));
        let last = exported["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["name"] == Stream::ToolEvents.name())
            .unwrap();
        assert_eq!(
            last["records"].as_array().unwrap().last().unwrap()["status"],
            "completed"
        );
    }
}
#[test]
fn malformed_oversized_and_strict_authoritative_appends_stay_fail_closed() {
    let root = root();
    assert_eq!(
        append(root.path(), Stream::ToolEvents, b"not-json"),
        Err(Fault::InvalidRecord)
    );
    let mut oversized = record();
    oversized.resize(RECORD_BYTES, b' ');
    assert_eq!(
        append(root.path(), Stream::ToolEvents, &oversized),
        Err(Fault::InvalidRecord)
    );
    write(root.path(), Stream::ToolEvents.name(), b"invalid\n");
    assert_eq!(history(root.path()).unwrap_err(), Fault::Recovery);
    let ledger = root.path().join("approval.jsonl");
    super::super::context::append_line(&ledger, b"{}", 3).unwrap();
    assert!(super::super::context::append_line(&ledger, b"{}", 3).is_err());
    assert_eq!(std::fs::read(ledger).unwrap(), b"{}\n");
}
#[cfg(unix)]
#[test]
fn every_fixed_path_refuses_symlinks_hardlinks_and_public_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    for name in [
        LOCK,
        SUMMARY,
        STAGE,
        Stream::ToolEvents.name(),
        Stream::ToolEvents.previous(),
    ] {
        for kind in ["symlink", "hardlink", "permissions"] {
            let root = root();
            let outside = root.path().join("unrelated-private");
            std::fs::write(&outside, b"protected").unwrap();
            std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o600)).unwrap();
            let path = root.path().join(name);
            match kind {
                "symlink" => symlink(&outside, &path).unwrap(),
                "hardlink" => std::fs::hard_link(&outside, &path).unwrap(),
                "permissions" => {
                    std::fs::write(&path, b"{}\n").unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
                        .unwrap();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                append(root.path(), Stream::ToolEvents, b"{}"),
                Err(Fault::UnsafePath),
                "{name}/{kind}"
            );
            assert_eq!(std::fs::read(outside).unwrap(), b"protected");
        }
    }
}

#[test]
fn concurrent_local_writers_and_readers_serialize_without_faults() {
    use std::sync::{Arc, Barrier};
    let root = Arc::new(root());
    append(root.path(), Stream::ToolEvents, b"{}").unwrap();
    let barrier = Arc::new(Barrier::new(6));
    let mut workers = Vec::new();
    for worker in 0..6 {
        let root = Arc::clone(&root);
        let barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            barrier.wait();
            for _ in 0..50 {
                if worker < 4 {
                    append(root.path(), Stream::ToolEvents, b"{}").unwrap();
                } else {
                    assert_eq!(history(root.path()).unwrap().state, HistoryState::Complete);
                }
            }
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let history = history(root.path()).unwrap();
    assert_eq!(history.state, HistoryState::Complete);
    assert_eq!(history.streams[0].current.records, 201);
    assert_eq!(history.streams[0].evicted_records, Some(0));
}
