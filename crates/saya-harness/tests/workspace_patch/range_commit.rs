use super::*;

/// Happy-path splice shapes on a fixed example: mid-file replace, pure
/// insertion (empty range), deletion (empty replacement), whole-file range,
/// and the append shape an empty range at EOF with a matching precondition
/// expresses without a second code path.
#[test]
fn patch_range_splices_mid_insert_delete_and_eof_append_shapes() {
    let sandbox = Sandbox::new("shapes");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();

    sandbox
        .ws
        .patch_range("victim.txt", 6..11, 11, b"there")
        .unwrap();
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"hello there");

    sandbox
        .ws
        .patch_range("victim.txt", 5..5, 11, b",")
        .unwrap();
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"hello, there");

    sandbox.ws.patch_range("victim.txt", 5..6, 12, b"").unwrap();
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"hello there");

    let len = bytes_of(&sandbox.ws, "victim.txt").len() as u64;
    sandbox
        .ws
        .patch_range("victim.txt", len..len, len, b"!")
        .unwrap();
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"hello there!");

    let full = bytes_of(&sandbox.ws, "victim.txt");
    let len = full.len() as u64;
    sandbox
        .ws
        .patch_range("victim.txt", 0..len, len, b"replaced")
        .unwrap();
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"replaced");
}

/// The guarded patch: a caller that observed the file's content digest and
/// pins it commits when the fresh re-read still hashes to that digest.
#[test]
fn guarded_patch_with_matching_digest_commits() {
    let sandbox = Sandbox::new("guarded-ok");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();
    assert_eq!(observed.size, 11);
    assert!(!observed.truncated);

    sandbox
        .ws
        .patch_range_guarded("victim.txt", 6..11, 11, &observed.digest, b"there")
        .expect("patch with the observed digest must commit");
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"hello there");
}

/// The same-size stale edit a length precondition cannot see: the file was
/// replaced with equal-length content between the observation and the patch,
/// so the size still matches and only the digest can refuse. The refusal
/// must leave the newer bytes in place and create no temp file at all.
#[test]
fn guarded_patch_refuses_same_size_changed_content() {
    let sandbox = Sandbox::new("guarded-stale");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();
    sandbox.ws.write("victim.txt", b"HELLO WORLD").unwrap();

    let error = sandbox
        .ws
        .patch_range_guarded("victim.txt", 6..11, 11, &observed.digest, b"there")
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

/// The append shape (empty range at EOF) checks the digest like any other
/// guarded patch: with the observed digest it commits, and against a stale
/// digest — content replaced same-size, so the positional precondition still
/// passes — it refuses and leaves the bytes unchanged.
#[test]
fn guarded_append_at_eof_checks_the_digest() {
    let sandbox = Sandbox::new("guarded-append");
    sandbox.ws.write("victim.txt", b"hello").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();

    sandbox
        .ws
        .patch_range_guarded("victim.txt", 5..5, 5, &observed.digest, b"!")
        .expect("append with the observed digest must commit");
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"hello!");

    let before = sandbox.ws.read("victim.txt", 4096).unwrap();
    sandbox.ws.write("victim.txt", b"WORLD!").unwrap();
    let error = sandbox
        .ws
        .patch_range_guarded("victim.txt", 6..6, 6, &before.digest, b"?")
        .expect_err("stale-digest append must refuse");
    assert!(
        matches!(&error, HarnessError::ContentChanged { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), b"WORLD!");
}

/// An interrupted commit leaves the original, never a partial file: a writer
/// thread hammers patches while a reader samples, and every observed content
/// is either the original or one whole patched result.
#[test]
fn the_commit_is_atomic() {
    let outer = std::env::temp_dir().join(format!("saya-patch-atomic-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outer);
    fs::create_dir_all(outer.join("ws")).unwrap();
    let ws = Workspace::open(&outer.join("ws")).unwrap();
    let original = vec![b'a'; 4096];
    ws.write("shared.txt", &original).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let writer = {
        let ws = ws.clone();
        let stop = Arc::clone(&stop);
        thread::spawn(move || {
            // Fixed-size replacement: every commit keeps the 4096-byte
            // length, so the reader's whole-commit set is exactly three
            // values — the original and the two patched heads.
            let mut toggle = false;
            while !stop.load(Ordering::Relaxed) {
                let current = fs::read(ws.root().join("shared.txt")).unwrap_or_default();
                let len = current.len() as u64;
                let replacement: &[u8] = if toggle { b"bbbbbbbb" } else { b"cccccccc" };
                toggle = !toggle;
                // A concurrent patch racing ours trips the precondition —
                // correctly, since the size moved under us. Anything else is
                // unexpected.
                match ws.patch_range("shared.txt", 0..8, len, replacement) {
                    Ok(()) => {}
                    Err(HarnessError::LengthMismatch { .. })
                    | Err(HarnessError::RangeOutOfBounds { .. })
                    | Err(HarnessError::IdentityChanged { .. })
                    | Err(HarnessError::Io { .. }) => {}
                    Err(other) => panic!("unexpected patch failure: {other:?}"),
                }
            }
        })
    };

    let mut head_b = original.clone();
    head_b[..8].copy_from_slice(b"bbbbbbbb");
    let mut head_c = original.clone();
    head_c[..8].copy_from_slice(b"cccccccc");
    for _ in 0..5_000 {
        match fs::read(ws.root().join("shared.txt")) {
            Ok(bytes) => {
                assert!(
                    bytes == original || bytes == head_b || bytes == head_c,
                    "observed a content that is no whole commit"
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => panic!("unexpected read failure: {error:?}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    let _ = fs::remove_dir_all(&outer);
}
