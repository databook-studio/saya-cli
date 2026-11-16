use super::*;

/// Eight writers race one guarded patch of the same range, each with a
/// different same-size replacement, all pinning the digest observed before
/// the spawn. The commit gate serializes the fresh-read → compare → publish
/// window, so exactly one commit is admitted, the other seven report
/// `ContentChanged`, and the file ends as one whole patched result.
#[test]
fn concurrent_guarded_writers_admit_exactly_one() {
    let sandbox = Sandbox::new("guarded-race");
    sandbox.ws.write("victim.txt", b"....abcd....").unwrap();
    let observed = sandbox.ws.read("victim.txt", 4096).unwrap();

    let replacements: [&[u8]; 8] = [
        b"AAAA", b"BBBB", b"CCCC", b"DDDD", b"EEEE", b"FFFF", b"GGGG", b"HHHH",
    ];
    let barrier = Arc::new(Barrier::new(replacements.len()));
    let handles: Vec<_> = replacements
        .iter()
        .copied()
        .map(|replacement| {
            let ws = sandbox.ws.clone();
            let barrier = Arc::clone(&barrier);
            let digest = observed.digest.clone();
            thread::spawn(move || {
                barrier.wait();
                ws.patch_range_guarded("victim.txt", 4..8, 12, &digest, replacement)
            })
        })
        .collect();

    let (mut wins, mut stale) = (0usize, 0usize);
    for handle in handles {
        match handle.join().unwrap() {
            Ok(()) => wins += 1,
            Err(HarnessError::ContentChanged { .. }) => stale += 1,
            Err(other) => panic!("unexpected concurrent failure: {other:?}"),
        }
    }
    assert_eq!(wins, 1, "exactly one of the eight writers must commit");
    assert_eq!(stale, 7, "the seven losers must all report ContentChanged");

    let final_bytes = bytes_of(&sandbox.ws, "victim.txt");
    assert!(
        replacements.iter().any(|replacement| {
            let mut whole = b"....abcd....".to_vec();
            whole[4..8].copy_from_slice(replacement);
            final_bytes == whole
        }),
        "committed content must be one whole patched result: {final_bytes:?}"
    );
}

/// A symlink at the final component is refused: the anchored walk's
/// no-follow scan fires before the patch body ever runs. Regression pin for
/// the seam, not for one line — deleting only the patch's own check still
/// refuses via the walk, and deleting only `O_NOFOLLOW` still refuses via
/// the walk's scan on this path (both verified with the probe; see
/// REPORT.md). The no-follow open's own proof is the pre-existing
/// `toctou_symlink_swap_never_serves_outside_content` race, which plants the
/// link *after* the scan.
#[cfg(unix)]
#[test]
fn a_final_component_symlink_is_refused_at_open() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("link-open");
    sandbox.ws.write("victim.txt", INSIDE).unwrap();
    let before = bytes_of(&sandbox.ws, "victim.txt");
    let outside = sandbox.outside_path("outside.txt");
    fs::write(&outside, OUTSIDE).unwrap();
    symlink(&outside, sandbox.root().join("link")).unwrap();

    let error = sandbox
        .ws
        .patch_range("link", 0..1, OUTSIDE.len() as u64, b"x")
        .expect_err("link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(fs::read(&outside).unwrap(), OUTSIDE);
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
}

/// swap: replacing an intermediate directory with a symlink to outside while
/// a patch runs either patches the real directory or refuses with a typed
/// error — outside content is never served and nothing lands outside.
#[cfg(unix)]
#[test]
fn a_toctou_swap_is_reported_not_hidden() {
    use std::os::unix::fs::symlink;

    let sandbox = Sandbox::new("toctou-patch");
    sandbox.ws.write("sub/dir/nested.txt", INSIDE).unwrap();
    let outside_dir = sandbox.outer.join("outside-dir");
    fs::create_dir_all(outside_dir.join("dir")).unwrap();
    fs::write(outside_dir.join("dir/nested.txt"), OUTSIDE).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let hostile = {
        let stop = Arc::clone(&stop);
        let root = sandbox.root().to_path_buf();
        let outside_dir = outside_dir.clone();
        thread::spawn(move || {
            let real = root.join("sub");
            let stage = root.join("sub.stage");
            while !stop.load(Ordering::Relaxed) {
                if fs::rename(&real, &stage).is_ok() {
                    let _ = symlink(&outside_dir, &real);
                    let _ = fs::remove_file(&real);
                    if fs::rename(&stage, &real).is_err() {
                        let _ = fs::remove_dir_all(&real);
                        let _ = fs::remove_file(&real);
                        let _ = fs::rename(&stage, &real);
                    }
                } else {
                    let _ = fs::remove_dir_all(&real);
                    let _ = fs::remove_file(&real);
                    let _ = fs::rename(&stage, &real);
                }
            }
        })
    };

    for round in 0..5_000 {
        // The pre-patch measurement is deliberately by raw path: it may race
        // the swapper and observe outside bytes, which is exactly what the
        // positional precondition exists to catch — a stale `expected_len`
        // refuses rather than splicing against moved offsets.
        let current = fs::read(sandbox.root().join("sub/dir/nested.txt"));
        let Ok(current) = current else {
            continue;
        };
        let len = current.len() as u64;
        match sandbox
            .ws
            .patch_range("sub/dir/nested.txt", 0..len.min(1), len, b"Z")
        {
            Ok(()) => {
                // Verified through the contained read, never by raw path: a
                // raw read here could sample the swapped window itself and
                // blame the patch for the swapper's bytes.
                match sandbox.ws.read("sub/dir/nested.txt", 4096) {
                    Ok(file) => assert!(
                        file.bytes == INSIDE
                            || file.bytes.len() == INSIDE.len() && file.bytes.starts_with(b"Z"),
                        "round {round}: patched content must derive from inside bytes"
                    ),
                    Err(error) => assert!(is_rejection(&error), "round {round}: {error}"),
                }
            }
            Err(error) => assert!(is_rejection(&error), "round {round}: {error}"),
        }
        // Nothing the patch did may have landed outside the root: no new
        // names beside the bait, bait bytes untouched.
        let strays: Vec<String> = fs::read_dir(outside_dir.join("dir"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| name != "nested.txt" && !name.starts_with(".saya-tmp-"))
            .collect();
        assert!(
            strays.is_empty(),
            "round {round}: a patch landed outside: {strays:?}"
        );
        assert_eq!(
            fs::read(outside_dir.join("dir/nested.txt")).unwrap(),
            OUTSIDE,
            "round {round}: outside bait untouched"
        );
    }
    stop.store(true, Ordering::Relaxed);
    hostile.join().unwrap();
}
