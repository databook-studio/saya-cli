use super::*;

#[cfg(unix)]
#[test]
fn toctou_symlink_swap_never_serves_outside_content() {
    // The hostile pass keeps `victim` a *regular file* almost all the time
    // and plants a symlink to a file outside the workspace for only a
    // microsecond-scale window per pass. The walk therefore sees a regular
    // file; the only way this attack harms the reader is if the swap lands
    // between the pre-open scan and the open — which the no-follow open is
    // the guard against.
    let sandbox = Sandbox::new("toctou-symlink");
    sandbox.plant();
    let victim = sandbox.root().join("victim");
    fs::write(&victim, INSIDE).unwrap();
    let outside = sandbox.outside_path("outside.txt");
    let stage = victim.with_extension("stage");
    let link = victim.with_extension("link");

    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicUsize::new(0));
    let hostile = {
        let stop = Arc::clone(&stop);
        let swaps = Arc::clone(&swaps);
        let victim = victim.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                let _ = fs::remove_file(&link);
                let _ = std::os::unix::fs::symlink(&outside, &link);
                let _ = fs::write(&stage, INSIDE);
                let _ = fs::rename(&stage, &victim);
                let _ = fs::rename(&link, &victim);
                let _ = fs::write(&stage, INSIDE);
                let _ = fs::rename(&stage, &victim);
                swaps.fetch_add(1, Ordering::Relaxed);
            }
        })
    };

    let mut ok = 0usize;
    let mut caught = 0usize;
    for _ in 0..30_000 {
        match sandbox.ws.read("victim", 4096) {
            Ok(file) => {
                ok += 1;
                assert_eq!(file.bytes, INSIDE, "outside content must never be served");
            }
            Err(error) => {
                assert!(is_rejection(&error), "{error}");
                caught += 1;
            }
        }
    }
    stop.store(true, Ordering::Relaxed);
    hostile.join().unwrap();
    assert!(
        swaps.load(Ordering::Relaxed) > 0,
        "hostile thread must have raced at all"
    );
    assert!(
        caught > 0,
        "the no-follow open must catch the swap (ok={ok}, caught={caught})"
    );
    let _ = fs::remove_dir_all(&sandbox.outer);
}

#[test]
fn toctou_file_swap_is_caught_by_the_identity_check() {
    // The hostile pass replaces the final component with a *different regular
    // file* (same directory, different inode). Pre-scan and open then disagree
    // on (dev, ino); the post-open identity check is the guard.
    let (_, caught, swaps) = race_final_component("toctou-file", 30_000, |stage, _| {
        fs::write(stage, INSIDE).unwrap();
    });
    assert!(swaps > 0, "hostile thread must have raced at all");
    assert!(
        caught > 0,
        "the identity check must catch the swap (caught={caught})"
    );
}

#[test]
fn rename_race_reads_return_whole_content_or_a_typed_refusal() {
    let sandbox = Sandbox::new("rename-race");
    sandbox.plant();
    let victim = sandbox.root().join("victim");
    fs::write(&victim, INSIDE).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let mut hostiles = Vec::new();
    for round in 0..2 {
        let stop = Arc::clone(&stop);
        let victim = victim.clone();
        hostiles.push(thread::spawn(move || {
            let other = victim.with_extension(format!("race-{round}"));
            while !stop.load(Ordering::Relaxed) {
                if fs::rename(&victim, &other).is_ok() {
                    let _ = fs::rename(&other, &victim);
                }
            }
        }));
    }

    for _ in 0..5_000 {
        match sandbox.ws.read("victim", 4096) {
            Ok(file) => assert_eq!(file.bytes, INSIDE, "reads must be whole, never torn"),
            Err(error) => assert!(is_rejection(&error), "{error}"),
        }
    }
    stop.store(true, Ordering::Relaxed);
    for hostile in hostiles {
        hostile.join().unwrap();
    }
}

#[test]
fn concurrent_writes_leave_one_whole_content_never_a_mix() {
    // Built without the Drop guard: writer threads must own the workspace,
    // and the cleanup runs after they are joined.
    let outer = std::env::temp_dir().join(format!("saya-ws-atomic-{}", std::process::id()));
    let _ = fs::remove_dir_all(&outer);
    fs::create_dir_all(outer.join("ws")).unwrap();
    let ws = Workspace::open(&outer.join("ws")).unwrap();
    let payload_a = vec![b'a'; 4096];
    let payload_b = vec![b'b'; 4096];

    let stop = Arc::new(AtomicBool::new(false));
    let mut writers = Vec::new();
    for payload in [payload_a.clone(), payload_b.clone()] {
        let ws = ws.clone();
        let stop = Arc::clone(&stop);
        writers.push(thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                // A second writer replacing this file microseconds after our
                // rename trips the post-write identity check — correctly, since
                // the bytes at that path are no longer the ones we placed. The
                // workspace documents a single writer (the engine), so that is
                // a caveat rather than a fault; what must never happen is a
                // torn file, which is what the reader below checks.
                match ws.write("shared.txt", &payload) {
                    Ok(()) => {}
                    Err(HarnessError::IdentityChanged { .. }) => {}
                    // Windows cannot replace a file while another thread has
                    // it open. The failed rename leaves the destination whole,
                    // which satisfies this test's atomicity contract.
                    Err(HarnessError::Io {
                        context, source, ..
                    }) if cfg!(windows)
                        && context == "replace workspace file"
                        && source.raw_os_error() == Some(32) => {}
                    Err(other) => panic!("unexpected write failure: {other:?}"),
                }
            }
        }));
    }
    // Sample the file while writes continue. Waiting a fixed 50ms for the
    // first rename to land was the original shape and it failed on a loaded
    // machine, where two threads had not finished an atomic write in that
    // window — a timing assumption standing in for the condition the test
    // actually needs, which is "a write has landed". Poll for that instead;
    // the writers are still running when the read happens either way, which
    // is the property under test.
    let path = ws.root().join("shared.txt");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    let observed = loop {
        if let Ok(bytes) = fs::read(&path) {
            break bytes;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no writer completed an atomic write within 30s"
        );
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    stop.store(true, Ordering::Relaxed);
    for writer in writers {
        writer.join().unwrap();
    }
    assert!(
        observed == payload_a || observed == payload_b,
        "a reader must observe one whole write, never interleaved bytes"
    );
    let _ = fs::remove_dir_all(&outer);
}
