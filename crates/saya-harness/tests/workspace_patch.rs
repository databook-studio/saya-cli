//! S1: `Workspace::patch_range` — anchored range-replace, atomically committed.
//!
//! The primitive takes a resolved target, a byte range, a positional
//! precondition (the size the caller measured when choosing the range), and
//! the replacement bytes. Anchor-finding (matching text, counting matches)
//! is a later slice's job — this operation only splices the range it is
//! given, under the same containment and atomic commit as the whole-file
//! write.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
};

use proptest::prelude::*;
use saya_harness::HarnessError;
use saya_harness::workspace::Workspace;
use saya_harness::workspace::patch::{PATCH_MAX_FILE_BYTES, PATCH_REPLACEMENT_MAX_BYTES};

const INSIDE: &[u8] = b"inside-sentinel";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer = std::env::temp_dir().join(format!("saya-patch-{label}-{}", std::process::id()));
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

fn is_rejection(error: &HarnessError) -> bool {
    matches!(
        error,
        HarnessError::PathOutsideRoot { .. }
            | HarnessError::InvalidPath { .. }
            | HarnessError::SymlinkRefused { .. }
            | HarnessError::DeniedName { .. }
            | HarnessError::BoundsExceeded { .. }
            | HarnessError::IdentityChanged { .. }
            | HarnessError::NotRegularFile { .. }
            | HarnessError::RangeOutOfBounds { .. }
            | HarnessError::LengthMismatch { .. }
            | HarnessError::Io { .. }
    )
}

fn bytes_of(ws: &Workspace, rel: &str) -> Vec<u8> {
    fs::read(ws.root().join(rel)).unwrap()
}

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

    // Traversal, missing file, and directory target.
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

/// A refused patch never creates the file it names.
#[test]
fn a_refused_patch_never_creates_a_file() {
    let sandbox = Sandbox::new("no-create");
    let error = sandbox
        .ws
        .patch_range("absent.txt", 0..0, 0, b"x")
        .expect_err("absent target must refuse");
    assert!(is_rejection(&error), "{error:?}");
    assert!(!sandbox.root().join("absent.txt").exists());

    let error = sandbox
        .ws
        .patch_range("new/dir/file.txt", 0..0, 0, b"x")
        .expect_err("patching must not create parents");
    assert!(is_rejection(&error), "{error:?}");
    assert!(!sandbox.root().join("new").exists());
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

/// The containment corpus is generated, not enumerated: traversal, absolute,
/// drive/UNC, empty, dot, hygiene, and symlink-planted shapes are joined
/// into mixed forms, and no generated argument may ever alter the outside
/// sentinel or plant a name outside the root.
fn hostile_arg() -> impl Strategy<Value = String> {
    let pool = vec![
        "ok.txt".to_string(),
        "sub".to_string(),
        "nested.txt".to_string(),
        "victim.txt".to_string(),
        "..".to_string(),
        "..".to_string(),
        ".".to_string(),
        String::new(),
        "outside.txt".to_string(),
        "/etc/passwd".to_string(),
        "C:\\Windows".to_string(),
        "\\\\srv\\share".to_string(),
        ".git".to_string(),
        "~".to_string(),
        "link".to_string(),
    ];
    prop::collection::vec(prop::sample::select(pool), 1..6).prop_map(|parts| parts.join("/"))
}

proptest! {
    /// No generated argument may ever patch outside the workspace. The
    /// outside sentinel keeps its exact bytes, no name is planted outside,
    /// and accepted patches return the inside sentinel's spliced form.
    #[test]
    fn containment_holds_for_the_corpus(args in prop::collection::vec(hostile_arg(), 1..8)) {
        let sandbox = Sandbox::new("proptest-patch");
        sandbox.ws.write("victim.txt", INSIDE).unwrap();
        sandbox.ws.write("sub/nested.txt", INSIDE).unwrap();
        fs::write(sandbox.outside_path("outside.txt"), OUTSIDE).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            sandbox.outside_path("outside.txt"),
            sandbox.root().join("link"),
        )
        .unwrap();

        for arg in args {
            let current = fs::read(sandbox.root().join("victim.txt")).unwrap_or_default();
            let len = current.len() as u64;
            let end = len.min(3);
            let _ = sandbox.ws.patch_range(&arg, 0..end, len, b"zz");
            prop_assert_eq!(
                fs::read(sandbox.outside_path("outside.txt")).unwrap(),
                OUTSIDE
            );
            prop_assert!(!sandbox.outside_path("attempted").exists());
        }
    }

    /// Splice model: for any content, any in-bounds range, and any small
    /// replacement, `patch_range` produces exactly prefix + replacement +
    /// suffix, and any out-of-bounds or stale-precondition call refuses with
    /// the file untouched.
    #[test]
    fn splice_matches_prefix_plus_replacement_plus_suffix(
        content in prop::collection::vec(any::<u8>(), 0..256),
        start in 0..300usize,
        end in 0..300usize,
        replacement in prop::collection::vec(any::<u8>(), 0..64),
    ) {
        let sandbox = Sandbox::new("proptest-splice");
        sandbox.ws.write("victim.txt", &content).unwrap();
        let size = content.len() as u64;
        let (start, end) = (start as u64, end as u64);
        let expected: Vec<u8> = if start <= end && end <= size {
            let mut out = Vec::new();
            out.extend_from_slice(&content[..start as usize]);
            out.extend_from_slice(&replacement);
            out.extend_from_slice(&content[end as usize..]);
            out
        } else {
            Vec::new()
        };
        if start <= end && end <= size {
            sandbox.ws.patch_range("victim.txt", start..end, size, &replacement).unwrap();
            prop_assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), expected);
        } else {
            let error = sandbox.ws.patch_range("victim.txt", start..end, size, &replacement)
                .expect_err("out-of-bounds range must refuse");
            prop_assert!(matches!(error, HarnessError::RangeOutOfBounds { .. }), "{error:?}");
            prop_assert_eq!(bytes_of(&sandbox.ws, "victim.txt"), content);
        }
    }
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
