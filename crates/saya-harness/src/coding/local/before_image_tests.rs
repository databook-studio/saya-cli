//! The before-image store's contract: first touch wins, bounds are recorded
//! rather than raised, secrets keep no bytes, and a torn index line loses only
//! that line.

use std::fs;
use std::io::Write as _;
use std::path::PathBuf;

use saya_types::WorkspacePath;

use super::before_image::{
    BeforeImageStore, BeforeState, FirstTouch, MAX_BEFORE_IMAGE_FILE_BYTES,
    MAX_BEFORE_IMAGE_INDEX_BYTES, MAX_BEFORE_IMAGE_PATHS, UnrecordedReason,
};

struct Sandbox {
    records: PathBuf,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let records = std::env::temp_dir().join(format!(
            "saya-before-image-{label}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&records);
        fs::create_dir_all(&records).expect("sandbox records must create");
        Self { records }
    }

    fn store(&self) -> BeforeImageStore {
        BeforeImageStore::open(&self.records).expect("the store opens")
    }

    fn store_root(&self) -> PathBuf {
        self.records.join("before-images")
    }

    fn object_files(&self) -> Vec<PathBuf> {
        let objects = self.store_root().join("objects");
        match fs::read_dir(objects) {
            Ok(entries) => entries
                .map(|entry| entry.expect("object entry").path())
                .collect(),
            Err(_) => Vec::new(),
        }
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.records);
    }
}

fn path(value: &str) -> WorkspacePath {
    WorkspacePath::try_from(value.to_owned()).expect("a valid workspace path")
}

fn file_state(entry: &super::before_image::BeforeImageEntry) -> (&str, u64, bool) {
    match &entry.state {
        BeforeState::File {
            sha256,
            size,
            executable,
        } => (sha256.as_str(), *size, *executable),
        other => panic!("expected a recorded file, got {other:?}"),
    }
}

#[test]
fn first_touch_wins() {
    let sandbox = Sandbox::new("first-touch");
    let store = sandbox.store();
    let path = path("src/lib.rs");

    assert_eq!(
        store
            .record_first_touch(&path, Some(b"first"), false, 1)
            .expect("first touch records"),
        FirstTouch::Recorded
    );
    assert_eq!(
        store
            .record_first_touch(&path, Some(b"second"), false, 2)
            .expect("second touch is a no-op"),
        FirstTouch::AlreadyRecorded
    );

    let entries = store.entries().expect("entries read");
    assert_eq!(entries.len(), 1, "one record per path");
    let (sha256, size, executable) = file_state(&entries[0]);
    assert_eq!(sha256, saya_types::sha256_hex(b"first"));
    assert_eq!(size, 5);
    assert!(!executable);
    assert_eq!(
        store.read_image(&entries[0]).expect("image reads"),
        Some(b"first".to_vec()),
        "the first bytes win"
    );
}

#[test]
fn absent_then_forget_leaves_no_entry() {
    let sandbox = Sandbox::new("forget");
    let store = sandbox.store();
    let path = path("created.txt");

    assert_eq!(
        store
            .record_first_touch(&path, None, false, 1)
            .expect("absent touch records"),
        FirstTouch::Recorded
    );
    assert_eq!(store.entries().expect("entries read").len(), 1);
    store.forget(path.as_str()).expect("forget succeeds");
    assert!(
        store.entries().expect("entries read").is_empty(),
        "a forgotten first touch leaves no entry"
    );
}

#[test]
fn over_file_bound_is_unrecorded_not_error() {
    let sandbox = Sandbox::new("file-bound");
    let store = sandbox.store();
    let path = path("huge.bin");
    let before = vec![0u8; MAX_BEFORE_IMAGE_FILE_BYTES as usize + 1];

    assert_eq!(
        store
            .record_first_touch(&path, Some(&before), false, 1)
            .expect("a bound is not an error"),
        FirstTouch::Unrecorded(UnrecordedReason::OverFileBound)
    );
    let entries = store.entries().expect("entries read");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].state,
        BeforeState::Unrecorded(UnrecordedReason::OverFileBound)
    );
    assert!(
        sandbox.object_files().is_empty(),
        "no object is written for an over-bound file"
    );
}

#[test]
fn over_path_bound_sets_overflowed() {
    let sandbox = Sandbox::new("path-bound");
    let store = sandbox.store();
    for index in 0..MAX_BEFORE_IMAGE_PATHS {
        assert_eq!(
            store
                .record_first_touch(&path(&format!("f{index}.txt")), None, false, 1)
                .expect("within the path bound"),
            FirstTouch::Recorded
        );
    }
    assert_eq!(
        store
            .record_first_touch(&path("one-too-many.txt"), None, false, 1)
            .expect("a bound is not an error"),
        FirstTouch::Unrecorded(UnrecordedReason::OverPathBound)
    );
    assert!(
        store.overflowed(),
        "the store reports the bound was reached"
    );
    assert_eq!(
        store.entries().expect("entries read").len(),
        MAX_BEFORE_IMAGE_PATHS,
        "the path bound is never exceeded"
    );
    let index = sandbox.store_root().join("index.jsonl");
    let size = fs::metadata(&index).expect("the index exists").len();
    assert!(
        size <= MAX_BEFORE_IMAGE_INDEX_BYTES as u64,
        "the index stays within its byte bound: {size}"
    );
}

#[test]
fn sensitive_path_stores_no_bytes() {
    let sandbox = Sandbox::new("sensitive");
    let store = sandbox.store();
    let path = path(".env");

    assert_eq!(
        store
            .record_first_touch(&path, Some(b"SECRET=1"), false, 1)
            .expect("a sensitive path is not an error"),
        FirstTouch::Unrecorded(UnrecordedReason::Sensitive)
    );
    let entries = store.entries().expect("entries read");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].state,
        BeforeState::Unrecorded(UnrecordedReason::Sensitive)
    );
    assert!(
        sandbox.object_files().is_empty(),
        "a sensitive path keeps no bytes"
    );
}

#[test]
fn corrupt_object_is_refused() {
    let sandbox = Sandbox::new("corrupt");
    let store = sandbox.store();
    let path = path("a.txt");
    store
        .record_first_touch(&path, Some(b"hello"), false, 1)
        .expect("records");
    let entries = store.entries().expect("entries read");
    let (sha256, _, _) = file_state(&entries[0]);
    let object = sandbox.store_root().join("objects").join(sha256);
    let mut bytes = fs::read(&object).expect("the object exists");
    bytes[0] ^= 0xff;
    fs::write(&object, &bytes).expect("flip one byte");

    assert!(
        store.read_image(&entries[0]).is_err(),
        "a digest mismatch is refused, never served"
    );
}

#[test]
fn index_survives_reopen_and_torn_last_line() {
    let sandbox = Sandbox::new("reopen");
    let store = sandbox.store();
    store
        .record_first_touch(&path("a.txt"), Some(b"alpha"), false, 1)
        .expect("records a");
    store
        .record_first_touch(&path("b.txt"), Some(b"beta"), false, 2)
        .expect("records b");
    let index = sandbox.store_root().join("index.jsonl");
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&index)
        .expect("the index opens");
    file.write_all(b"{\"event\":\"touch\",\"path\":\"torn")
        .expect("a torn tail writes");
    drop(file);

    let reopened = sandbox.store();
    let entries = reopened.entries().expect("a torn tail is ignored");
    assert_eq!(entries.len(), 2, "earlier entries survive");
    assert_eq!(entries[0].path, "a.txt");
    assert_eq!(entries[1].path, "b.txt");
    assert_eq!(
        reopened.read_image(&entries[0]).expect("image reads"),
        Some(b"alpha".to_vec())
    );
}
