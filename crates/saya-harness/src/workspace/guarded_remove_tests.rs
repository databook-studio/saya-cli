//! Adversarial tests for the guarded removal: the digest guard, the entry
//! kinds it refuses, the containment refusals, and the race contract among
//! Saya's own serialized writers. Mirrors the guarded whole-file write's
//! tests in `tests/workspace_patch.rs`.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use crate::HarnessError;
use crate::workspace::{MAX_IO_BYTES, Workspace};

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

/// The happy path: a removal whose fresh re-read hashes to the observed
/// digest deletes exactly that file, leaves its parent directory in place,
/// and leaves no temp file behind.
#[test]
fn remove_guarded_removes_a_matching_file_and_keeps_the_parent() {
    let sandbox = Sandbox::new("rm-ok");
    sandbox.ws.write("dir/victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("dir/victim.txt", 4096).unwrap();

    sandbox
        .ws
        .remove_guarded("dir/victim.txt", &observed.digest)
        .expect("removal with the observed digest must commit");
    assert!(!sandbox.root().join("dir/victim.txt").exists());
    assert!(
        sandbox.root().join("dir").is_dir(),
        "the parent directory must remain"
    );
    let strays: Vec<String> = fs::read_dir(sandbox.root().join("dir"))
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    assert!(strays.is_empty(), "strays after removal: {strays:?}");
}

/// A wrong digest refuses with `ContentChanged` naming the expected and the
/// actual digests, and the file stays byte-identical — nothing is deleted.
#[test]
fn remove_guarded_refuses_a_wrong_digest_and_preserves_the_bytes() {
    let sandbox = Sandbox::new("rm-wrong");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    sandbox
        .ws
        .write("sub/other.txt", b"different content")
        .unwrap();
    let other_digest = sandbox.ws.read("sub/other.txt", 4096).unwrap().digest;
    let before = bytes_of(&sandbox.ws, "victim.txt");

    let error = sandbox
        .ws
        .remove_guarded("victim.txt", &other_digest)
        .expect_err("a wrong digest must refuse");
    assert!(
        matches!(
            &error,
            HarnessError::ContentChanged { path, expected, current }
                if path == "victim.txt" && *expected == other_digest && !current.is_empty()
        ),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
    let strays = stray_names(&sandbox);
    assert!(strays.is_empty(), "stray files after refusal: {strays:?}");
}

/// Removal names an existing file only: a missing final component is the
/// typed `NotFound` — removal of nothing is not success — and the refusal
/// creates nothing.
#[test]
fn remove_guarded_refuses_a_missing_file_with_not_found() {
    let sandbox = Sandbox::new("rm-missing");

    let error = sandbox
        .ws
        .remove_guarded("absent.txt", EMPTY_DIGEST)
        .expect_err("missing target must refuse");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "absent.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("absent.txt").exists());

    let error = sandbox
        .ws
        .remove_guarded("new/dir/file.txt", EMPTY_DIGEST)
        .expect_err("missing parents must refuse too");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "new/dir/file.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("new").exists());
}

/// A directory at the path refuses as `NotRegularFile`, and the directory's
/// own content is untouched.
#[test]
fn remove_guarded_refuses_a_directory_target() {
    let sandbox = Sandbox::new("rm-dir");
    sandbox.ws.write("sub/nested.txt", b"keep me").unwrap();

    let error = sandbox
        .ws
        .remove_guarded("sub", EMPTY_DIGEST)
        .expect_err("directory target must refuse");
    assert!(
        matches!(error, HarnessError::NotRegularFile { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "sub/nested.txt"), b"keep me");
}

/// A symlink at the final component is refused without being followed or
/// removed: the target keeps its exact bytes and the link itself survives.
#[cfg(unix)]
#[test]
fn remove_guarded_refuses_a_final_component_symlink_and_spares_the_target() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("rm-link");
    let outside = sandbox.outside_path("outside.txt");
    fs::write(&outside, OUTSIDE).unwrap();
    symlink(&outside, sandbox.root().join("link")).unwrap();

    let error = sandbox
        .ws
        .remove_guarded("link", EMPTY_DIGEST)
        .expect_err("symlink target must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(&outside).unwrap(), OUTSIDE);
    assert!(
        sandbox
            .root()
            .join("link")
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link itself must not have been removed"
    );
}

/// A symlinked parent component is refused by the walk before the final
/// component is ever touched: nothing behind the link is removed.
#[cfg(unix)]
#[test]
fn remove_guarded_refuses_a_symlinked_parent_component() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("rm-linkdir");
    let outside_dir = sandbox.outer.join("outside-dir");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("inside.txt"), OUTSIDE).unwrap();
    symlink(&outside_dir, sandbox.root().join("linkdir")).unwrap();

    let error = sandbox
        .ws
        .remove_guarded("linkdir/inside.txt", EMPTY_DIGEST)
        .expect_err("symlinked parent must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(outside_dir.join("inside.txt")).unwrap(), OUTSIDE);
}

/// Traversal and absolute forms refuse before any filesystem contact, and a
/// real file beside them is untouched.
#[test]
fn remove_guarded_refuses_escaping_and_invalid_arguments() {
    let sandbox = Sandbox::new("rm-escape");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let before = bytes_of(&sandbox.ws, "victim.txt");

    for argument in ["../outside.txt", "/etc/passwd"] {
        let error = sandbox
            .ws
            .remove_guarded(argument, EMPTY_DIGEST)
            .expect_err("escaping argument must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "{argument} → {error:?}"
        );
    }
    let error = sandbox
        .ws
        .remove_guarded("", EMPTY_DIGEST)
        .expect_err("empty argument must refuse");
    assert!(
        matches!(error, HarnessError::InvalidPath { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
}

/// A file over the whole-file I/O bound refuses before anything is read or
/// removed: the typed bound names the size and the cap, and the file keeps
/// its bytes.
#[test]
fn remove_guarded_over_the_io_bound_is_refused_whole() {
    let sandbox = Sandbox::new("rm-bound");
    let huge = vec![b'x'; MAX_IO_BYTES + 1];
    fs::create_dir_all(sandbox.root().join("big")).unwrap();
    fs::write(sandbox.root().join("big/huge.bin"), &huge).unwrap();

    let error = sandbox
        .ws
        .remove_guarded("big/huge.bin", EMPTY_DIGEST)
        .expect_err("over-bound target must refuse");
    assert!(
        matches!(
            error,
            HarnessError::BoundsExceeded { found, max, .. }
                if found == huge.len() as u64 && max == MAX_IO_BYTES as u64
        ),
        "{error:?}"
    );
    assert_eq!(
        fs::read(sandbox.root().join("big/huge.bin")).unwrap().len(),
        huge.len()
    );
}

/// The stale removal a size check cannot see: the file changed between the
/// observation and the call, so only the digest refuses. The newer bytes
/// stay and no temp file is created anywhere.
#[test]
fn remove_guarded_refuses_content_changed_after_observation() {
    let sandbox = Sandbox::new("rm-stale");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();
    sandbox.ws.write("victim.txt", b"HELLO WORLD").unwrap();

    let error = sandbox
        .ws
        .remove_guarded("victim.txt", &observed.digest)
        .expect_err("changed content must refuse");
    assert!(
        matches!(
            &error,
            HarnessError::ContentChanged { path, expected, current }
                if path == "victim.txt" && *expected == observed.digest && !current.is_empty()
        ),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"HELLO WORLD");
    let strays = stray_names(&sandbox);
    assert!(strays.is_empty(), "stray files after refusal: {strays:?}");
}

/// Two Saya writers race one removal of the same file behind a barrier. The
/// commit gate serializes walk → verify → unlink, so exactly one is
/// admitted and the other reports `NotFound` — never both Ok, never a panic.
#[test]
fn concurrent_remove_guarded_admits_exactly_one() {
    let sandbox = Sandbox::new("rm-race");
    sandbox.ws.write("race.txt", b"shared bytes").unwrap();
    let observed = sandbox.ws.read("race.txt", 4096).unwrap();

    let barrier = Arc::new(Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let ws = sandbox.ws.clone();
            let barrier = Arc::clone(&barrier);
            let digest = observed.digest.clone();
            thread::spawn(move || {
                barrier.wait();
                ws.remove_guarded("race.txt", &digest)
            })
        })
        .collect();

    let (mut removed, mut absent) = (0usize, 0usize);
    for handle in handles {
        match handle.join().unwrap() {
            Ok(()) => removed += 1,
            Err(HarnessError::NotFound { .. }) => absent += 1,
            Err(other) => panic!("unexpected concurrent failure: {other:?}"),
        }
    }
    assert_eq!(removed, 1, "exactly one of the two removals must commit");
    assert_eq!(absent, 1, "the loser must report NotFound, not success");
    assert!(!sandbox.root().join("race.txt").exists());
}

/// Recovery's undo shape: after a successful removal the path is free again,
/// so a `create_new` on it succeeds whole.
#[test]
fn remove_guarded_allows_create_new_afterwards() {
    let sandbox = Sandbox::new("rm-recreate");
    sandbox.ws.write("victim.txt", b"hello").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();

    sandbox
        .ws
        .remove_guarded("victim.txt", &observed.digest)
        .expect("removal with the observed digest must commit");
    assert!(!sandbox.root().join("victim.txt").exists());

    sandbox
        .ws
        .create_new("victim.txt", b"fresh")
        .expect("create after removal must succeed");
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"fresh");
}
