use super::*;

/// `create_new` is create-if-absent: a missing file is created whole, and
/// missing parent directories come with it. Refusals are the next tests'
/// job; this one pins the happy path and the created tree.
#[test]
fn create_new_creates_a_missing_file_with_missing_parents() {
    let sandbox = Sandbox::new("create-new-ok");
    sandbox
        .ws
        .create_new("new/deep/file.txt", b"fresh bytes")
        .expect("create of a missing file with missing parents must succeed");
    assert_eq!(bytes_of(&sandbox.ws, "new/deep/file.txt"), b"fresh bytes");
}

/// A create that finds the target already there must refuse with the typed
/// `AlreadyExists` and leave the existing bytes exactly as they were — the
/// anti-clobber contract of create-if-absent.
#[test]
fn create_new_refuses_an_existing_file_and_preserves_it() {
    let sandbox = Sandbox::new("create-new-exists");
    sandbox.ws.write("victim.txt", b"original").unwrap();
    let before = bytes_of(&sandbox.ws, "victim.txt");

    let error = sandbox
        .ws
        .create_new("victim.txt", b"clobber")
        .expect_err("an existing file must refuse create");
    assert!(
        matches!(&error, HarnessError::AlreadyExists { path } if path == "victim.txt"),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
}

/// Every other kind of entry refuses a create too: a directory is the typed
/// `AlreadyExists`, and (unix) a symlink is refused by the shared walk
/// without being followed or replaced — the outside sentinel stays intact.
#[test]
fn create_new_refuses_an_existing_directory_and_symlink() {
    let sandbox = Sandbox::new("create-new-entries");
    sandbox.ws.write("sub/nested.txt", INSIDE).unwrap();

    let error = sandbox
        .ws
        .create_new("sub", b"x")
        .expect_err("directory target must refuse create");
    assert!(
        matches!(error, HarnessError::AlreadyExists { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "sub/nested.txt"), INSIDE);

    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = sandbox.outside_path("outside.txt");
        fs::write(&outside, OUTSIDE).unwrap();
        symlink(&outside, sandbox.root().join("link")).unwrap();
        let error = sandbox
            .ws
            .create_new("link", b"replacement")
            .expect_err("symlink target must refuse create");
        assert!(
            matches!(error, HarnessError::SymlinkRefused { .. }),
            "{error:?}"
        );
        assert_eq!(
            fs::read(&outside).unwrap(),
            OUTSIDE,
            "outside sentinel must stay intact"
        );
        assert!(
            sandbox
                .root()
                .join("link")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink(),
            "the link itself must not have been replaced"
        );
    }
}

/// Eight threads race one `create_new` of the same path behind a barrier,
/// each with different content. The commit gate serializes the
/// walk → existence check → publish window, so exactly one is admitted, the
/// other seven report `AlreadyExists`, and the file is one whole content —
/// the concurrent form of never clobbering.
#[test]
fn concurrent_create_new_admits_exactly_one() {
    let sandbox = Sandbox::new("create-new-race");
    let contents: [&[u8]; 8] = [
        b"AAAA", b"BBBB", b"CCCC", b"DDDD", b"EEEE", b"FFFF", b"GGGG", b"HHHH",
    ];
    let barrier = Arc::new(Barrier::new(contents.len()));
    let handles: Vec<_> = contents
        .iter()
        .copied()
        .map(|content| {
            let ws = sandbox.ws.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                ws.create_new("race.txt", content)
            })
        })
        .collect();

    let (mut wins, mut refused) = (0usize, 0usize);
    for handle in handles {
        match handle.join().unwrap() {
            Ok(()) => wins += 1,
            Err(HarnessError::AlreadyExists { .. }) => refused += 1,
            Err(other) => panic!("unexpected concurrent failure: {other:?}"),
        }
    }
    assert_eq!(wins, 1, "exactly one of the eight creates must be admitted");
    assert_eq!(refused, 7, "the seven losers must all report AlreadyExists");

    let final_bytes = bytes_of(&sandbox.ws, "race.txt");
    assert!(
        contents.iter().any(|content| final_bytes == *content),
        "the file must hold one whole created content: {final_bytes:?}"
    );
}
