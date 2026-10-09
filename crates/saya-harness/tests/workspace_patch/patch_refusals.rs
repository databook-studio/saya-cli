use super::*;

/// Every refusal path leaves the target byte-identical: stale precondition,
/// out-of-bounds range, over-bound replacement, traversal, symlink, missing
/// file, directory target, and oversized file.
#[test]
fn a_refused_patch_leaves_the_bytes_identical() {
    let sandbox = Sandbox::new("refused");
    sandbox.ws.write("victim.txt", b"hello world").unwrap();
    let before = bytes_of(&sandbox.ws, "victim.txt");

    // Stale size precondition: the caller measured 999 bytes, the file holds
    // 11, so the patch must refuse and touch nothing.
    let error = sandbox
        .ws
        .patch_range("victim.txt", 6..11, 999, b"there")
        .expect_err("stale precondition must refuse");
    assert!(
        matches!(error, HarnessError::LengthMismatch { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);

    // Out-of-bounds ranges: end past size, start past end, start past size.
    // (`11..6` literal would trip `clippy::reversed_empty_ranges`, so the
    // inverted case is spelled through variables.)
    let (inv_start, inv_end) = (11u64, 6u64);
    for range in [6..99u64, inv_start..inv_end, 12..12u64] {
        let error = sandbox
            .ws
            .patch_range("victim.txt", range.clone(), 11, b"x")
            .expect_err("out-of-bounds range must refuse");
        assert!(
            matches!(error, HarnessError::RangeOutOfBounds { .. }),
            "{range:?} → {error:?}"
        );
        assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);
    }

    // Over-bound replacement: refused whole, file untouched.
    let oversized = vec![b'x'; PATCH_REPLACEMENT_MAX_BYTES + 1];
    let error = sandbox
        .ws
        .patch_range("victim.txt", 6..11, 11, &oversized)
        .expect_err("over-bound replacement must refuse");
    assert!(
        matches!(error, HarnessError::BoundsExceeded { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);

    // Traversal, missing file, and directory target. The traversal pins the
    // escape variant, the absent file pins the typed `NotFound` (carrying
    // the workspace-relative name), and the directory pins
    // `NotRegularFile` — the same three properties the generic
    // `is_rejection` gate checked before, now spelled per case.
    for (rel, range) in [
        ("../outside.txt", 0..1u64),
        ("absent.txt", 0..0u64),
        ("sub", 0..0u64),
    ] {
        let error = sandbox
            .ws
            .patch_range(rel, range, 0, b"x")
            .expect_err("must refuse");
        assert!(is_rejection(&error), "{rel} → {error:?}");
    }
    assert!(matches!(
        sandbox
            .ws
            .patch_range("absent.txt", 0..0, 0, b"x")
            .expect_err("absent target must refuse"),
        HarnessError::NotFound { path } if path == "absent.txt"
    ));
    assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), before);

    // Symlinked target: refused, outside file untouched. The walk itself
    // already refuses links, so the check inside the patch is a second
    // opinion at the seam — but the byte-identity contract must hold at this
    // layer regardless of which one fires.
    #[cfg(unix)]
    {
        use std::os::unix::fs::symlink;
        let outside = sandbox.outside_path("outside.txt");
        fs::write(&outside, OUTSIDE).unwrap();
        symlink(&outside, sandbox.root().join("link")).unwrap();
        // The link points at a 33-byte file; any write through it would
        // change the outside bytes, and any splice of the link path would
        // prove the guard missing. Both the walk's verdict and the patch's
        // own check must refuse.
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
}

/// A refused patch never creates the file it names: the absent target pins
/// the typed `NotFound` (workspace-relative name), and a patch that would
/// need parent creation pins a refusal while creating no directory.
#[test]
fn a_refused_patch_never_creates_a_file() {
    let sandbox = Sandbox::new("no-create");
    let error = sandbox
        .ws
        .patch_range("absent.txt", 0..0, 0, b"x")
        .expect_err("absent target must refuse");
    assert!(
        matches!(&error, HarnessError::NotFound { path } if path == "absent.txt"),
        "{error:?}"
    );
    assert!(!sandbox.root().join("absent.txt").exists());

    let error = sandbox
        .ws
        .patch_range("new/dir/file.txt", 0..0, 0, b"x")
        .expect_err("patching must not create parents");
    assert!(is_rejection(&error), "{error:?}");
    assert!(!sandbox.root().join("new").exists());
}

/// Oversized targets refuse loudly with the size and the cap named, and the
/// file is untouched — never silently clipped.
#[test]
fn oversized_files_refuse_whole_with_size_and_cap() {
    let sandbox = Sandbox::new("too-large");
    let huge = vec![b'x'; (PATCH_MAX_FILE_BYTES + 1) as usize];
    fs::create_dir_all(sandbox.root().join("big")).unwrap();
    fs::write(sandbox.root().join("big/huge.bin"), &huge).unwrap();

    let error = sandbox
        .ws
        .patch_range("big/huge.bin", 0..1, huge.len() as u64, b"y")
        .expect_err("oversized target must refuse");
    assert!(
        matches!(error, HarnessError::BoundsExceeded { found, max, .. }
            if found == huge.len() as u64 && max == PATCH_MAX_FILE_BYTES),
        "{error:?}"
    );
    assert_eq!(
        fs::read(sandbox.root().join("big/huge.bin")).unwrap().len(),
        huge.len()
    );
}

/// Patching a directory refuses as a not-regular-file, byte-identical.
#[test]
fn patching_a_directory_refuses() {
    let sandbox = Sandbox::new("dir-target");
    sandbox.ws.write("sub/nested.txt", INSIDE).unwrap();
    let error = sandbox
        .ws
        .patch_range("sub", 0..0, 0, b"x")
        .expect_err("directory target must refuse");
    assert!(
        matches!(error, HarnessError::NotRegularFile { .. }),
        "{error:?}"
    );
    assert_eq!(bytes_of(&sandbox.ws, "sub/nested.txt"), INSIDE);
}
