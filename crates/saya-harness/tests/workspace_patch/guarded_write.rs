use super::*;

/// The guarded whole-file replace commits when the fresh re-read of the
/// existing file still hashes to the digest the caller observed.
#[test]
fn write_guarded_replaces_when_the_digest_matches() {
    let sandbox = Sandbox::new("guarded-write-ok");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();

    sandbox
        .ws
        .write_guarded("victim.txt", &observed.digest, b"replaced whole")
        .expect("replace with the observed digest must commit");
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"replaced whole");
}

/// The stale replace a size check cannot see: the file changed same-size
/// between observation and write, so only the digest refuses. The refusal
/// leaves the newer bytes in place and creates no temp file at all.
#[test]
fn write_guarded_refuses_same_size_changed_content() {
    let sandbox = Sandbox::new("guarded-write-stale");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();
    sandbox.ws.write("victim.txt", b"HELLO WORLD").unwrap();

    let error = sandbox
        .ws
        .write_guarded("victim.txt", &observed.digest, b"stale replacement")
        .expect_err("same-size stale content must refuse");
    assert!(
        matches!(
            &error,
            HarnessError::ContentChanged { path, expected, current }
                if path == "victim.txt" && *expected == observed.digest && !current.is_empty()
        ),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"HELLO WORLD");
    let strays: Vec<String> = fs::read_dir(sandbox.root())
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name != "victim.txt")
        .collect();
    assert!(strays.is_empty(), "stray files after refusal: {strays:?}");
}

/// A guarded replace names an existing file only: a missing final component
/// is the typed `NotFound` (creation is `create_new`'s job), and nothing is
/// created by the refusal.
#[test]
fn write_guarded_refuses_a_missing_file_with_not_found() {
    let sandbox = Sandbox::new("guarded-write-missing");
    let empty_digest = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    let error = sandbox
        .ws
        .write_guarded("absent.txt", empty_digest, b"x")
        .expect_err("missing target must refuse");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "absent.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("absent.txt").exists());

    let error = sandbox
        .ws
        .write_guarded("new/dir/file.txt", empty_digest, b"x")
        .expect_err("missing parents must refuse too");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "new/dir/file.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("new").exists());
}

/// Content over the whole-file write bound is refused before anything is
/// read or written: the typed bound names the size and the cap, and the
/// file is untouched.
#[test]
fn write_guarded_over_the_io_bound_is_refused_whole() {
    let sandbox = Sandbox::new("guarded-write-bound");
    sandbox.ws.write("victim.txt", b"keep").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();
    let before = bytes_of(&sandbox.ws, "victim.txt");

    let oversized = vec![b'x'; MAX_IO_BYTES + 1];
    let error = sandbox
        .ws
        .write_guarded("victim.txt", &observed.digest, &oversized)
        .expect_err("over-bound content must refuse");
    assert!(
        matches!(
            error,
            HarnessError::BoundsExceeded { found, max, .. }
                if found == (MAX_IO_BYTES + 1) as u64 && max == MAX_IO_BYTES as u64
        ),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
}
