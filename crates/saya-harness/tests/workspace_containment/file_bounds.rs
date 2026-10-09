use super::*;

#[cfg(windows)]
#[test]
fn overwriting_a_workspace_file_preserves_the_committed_temp_identity() {
    let sandbox = Sandbox::new("windows-overwrite");
    sandbox.ws.write("existing.txt", b"before").unwrap();

    sandbox.ws.write("existing.txt", b"after").unwrap();

    assert_eq!(
        sandbox.ws.read("existing.txt", 4096).unwrap().bytes,
        b"after"
    );
}

#[test]
#[cfg(unix)]
fn written_files_carry_no_execute_bits_ever() {
    use std::os::unix::fs::PermissionsExt;
    let sandbox = Sandbox::new("no-exec");
    sandbox.plant();

    // A pre-existing executable file must lose its execute bits on overwrite.
    let target = sandbox.root().join("script.sh");
    fs::write(&target, b"old").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();

    sandbox.ws.write("script.sh", INSIDE).unwrap();
    let mode = fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "writes land at 0600, never executable");

    sandbox.ws.write("fresh.txt", INSIDE).unwrap();
    let mode = fs::metadata(sandbox.root().join("fresh.txt"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0, "no execute bits on any written file");
}

#[test]
fn read_caps_are_honoured_with_a_visible_truncation_flag() {
    let sandbox = Sandbox::new("read-cap");
    let huge = vec![b'x'; 1 << 20];
    sandbox.ws.write("huge.txt", &huge).unwrap();

    let read = sandbox.ws.read("huge.txt", 1024).unwrap();
    assert_eq!(read.bytes.len(), 1024);
    assert_eq!(read.size, 1 << 20);
    assert!(read.truncated);

    let exact = sandbox.ws.read("huge.txt", 1 << 20).unwrap();
    assert!(!exact.truncated);
}

#[test]
fn an_oversized_file_read_refuses_before_hashing_it() {
    let sandbox = Sandbox::new("read-oversize-refusal");
    let target = sandbox.root().join("oversize.bin");
    let oversize = (MAX_IO_BYTES as u64) + 1;
    let file = fs::File::create(&target).expect("oversize fixture must plant");
    file.set_len(oversize).expect("sparse fixture must size");
    drop(file);

    let error = sandbox
        .ws
        .read("oversize.bin", 1024)
        .expect_err("a file past the I/O backstop must refuse before hashing");
    match error {
        HarnessError::BoundsExceeded { found, max, .. } => {
            assert_eq!(found, oversize);
            assert_eq!(max, MAX_IO_BYTES as u64);
        }
        other => panic!("the refusal must be the typed bound: {other:?}"),
    }
}

#[test]
fn a_file_at_the_io_backstop_still_reads_and_hashes_whole() {
    let sandbox = Sandbox::new("read-at-backstop");
    use sha2::{Digest, Sha256};
    let bytes = vec![b'q'; MAX_IO_BYTES];
    sandbox.ws.write("at-cap.bin", &bytes).unwrap();
    let read = sandbox.ws.read("at-cap.bin", 1024).unwrap();
    assert_eq!(read.bytes.len(), 1024);
    assert!(read.truncated);
    let mut expected = String::with_capacity(64);
    for byte in Sha256::digest(&bytes) {
        expected.push_str(&format!("{byte:02x}"));
    }
    assert_eq!(read.digest, expected);
}

#[test]
fn workspace_io_caps_cannot_be_widened_by_a_direct_caller() {
    let sandbox = Sandbox::new("io-cap-backstop");
    sandbox.ws.write("small.txt", b"ok").unwrap();

    let read = sandbox
        .ws
        .read("small.txt", (MAX_IO_BYTES + 1) as u64)
        .expect_err("a caller cannot widen the read cap");
    assert!(matches!(read, HarnessError::BoundsExceeded { .. }));

    let write = sandbox
        .ws
        .write("large.txt", &vec![b'x'; MAX_IO_BYTES + 1])
        .expect_err("a caller cannot widen the write cap");
    assert!(matches!(write, HarnessError::BoundsExceeded { .. }));

    let list = sandbox
        .ws
        .list("", MAX_LIST_ENTRIES + 1)
        .expect_err("a caller cannot widen the listing cap");
    assert!(matches!(list, HarnessError::BoundsExceeded { .. }));
}

#[test]
fn directory_listings_refuse_entry_floods() {
    let sandbox = Sandbox::new("list-flood");
    for i in 0..10 {
        sandbox.ws.write(&format!("f{i:02}.txt"), INSIDE).unwrap();
    }
    let listed = sandbox.ws.list("", 100).unwrap();
    assert_eq!(listed.len(), 10);
    let error = sandbox.ws.list("", 5).expect_err("flood must refuse");
    assert!(
        matches!(error, HarnessError::BoundsExceeded { .. }),
        "{error:?}"
    );
}

#[test]
fn directories_are_not_readable_as_files() {
    let sandbox = Sandbox::new("dir-read");
    sandbox.plant();
    let error = sandbox.ws.read("sub", 4096).expect_err("must refuse");
    assert!(
        matches!(error, HarnessError::NotRegularFile { .. }),
        "{error:?}"
    );
}
