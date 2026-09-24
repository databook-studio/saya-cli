//! The containment battery: every escape the plan's adversarial section
//! lists, each paired with the guard that must catch it. The happy path is
//! not the deliverable here — these tests exist so that neutralising any
//! single guard turns exactly the test paired with it red.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use proptest::prelude::*;
use saya_harness::{
    HarnessError,
    workspace::{MAX_IO_BYTES, MAX_LIST_ENTRIES, Workspace},
};

const INSIDE: &[u8] = b"inside-sentinel";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";

/// Keeps a sandbox directory alive for the test and removes it afterwards.
/// The workspace root sits one level down (`outer/ws`) so `../` escapes have
/// a real target (`outer/outside.txt`) to land on.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer = std::env::temp_dir().join(format!("saya-ws-{label}-{}", std::process::id()));
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

    fn plant(&self) {
        self.ws.write("ok.txt", INSIDE).unwrap();
        self.ws.write("sub/nested.txt", INSIDE).unwrap();
        fs::write(self.outside_path("outside.txt"), OUTSIDE).unwrap();
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
            | HarnessError::NotFound { .. }
            | HarnessError::Io { .. }
    )
}

#[test]
fn scratch_import_read_has_its_own_32_mib_cap_and_containment() {
    let sandbox = Sandbox::new("scratch-import-read");
    let medium = vec![b'x'; MAX_IO_BYTES + 1];
    fs::write(sandbox.root().join("medium.csv"), &medium).unwrap();
    assert!(matches!(
        sandbox.ws.read("medium.csv", MAX_IO_BYTES as u64),
        Err(HarnessError::BoundsExceeded { .. })
    ));
    let imported = sandbox.ws.read_for_scratch_import("medium.csv").unwrap();
    assert_eq!(imported.bytes, medium);
    assert!(!imported.truncated);

    let too_large = fs::File::create(sandbox.root().join("large.csv")).unwrap();
    too_large.set_len(32 * 1024 * 1024 + 1).unwrap();
    assert!(matches!(
        sandbox.ws.read_for_scratch_import("large.csv"),
        Err(HarnessError::BoundsExceeded { max, .. }) if max == 32 * 1024 * 1024
    ));
    assert!(is_rejection(
        &sandbox
            .ws
            .read_for_scratch_import("../outside.csv")
            .unwrap_err()
    ));
}

#[cfg(unix)]
#[test]
fn scratch_import_read_refuses_an_outside_symlink() {
    let sandbox = Sandbox::new("scratch-import-symlink");
    fs::write(sandbox.outside_path("outside.csv"), OUTSIDE).unwrap();
    std::os::unix::fs::symlink(
        sandbox.outside_path("outside.csv"),
        sandbox.root().join("link.csv"),
    )
    .unwrap();
    assert!(matches!(
        sandbox.ws.read_for_scratch_import("link.csv"),
        Err(HarnessError::SymlinkRefused { .. })
    ));
}

/// The hostile-argument corpus is generated, not enumerated: components are
/// drawn from a pool of traversal, absolute, drive/UNC, empty, dot, and
/// hygiene shapes, then joined into mixed forms.
fn hostile_arg() -> impl Strategy<Value = String> {
    let pool = vec![
        "ok.txt".to_string(),
        "sub".to_string(),
        "nested.txt".to_string(),
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
    ];
    prop::collection::vec(prop::sample::select(pool), 1..6).prop_map(|parts| parts.join("/"))
}

fn hostile_args() -> impl Strategy<Value = Vec<String>> {
    prop::collection::vec(hostile_arg(), 1..8)
}

proptest! {
    /// No generated argument may ever produce content from outside the
    /// workspace: accepted reads return the planted inside sentinel and every
    /// other outcome is a typed rejection.
    #[test]
    fn no_generated_argument_reads_outside_content(args in hostile_args()) {
        let sandbox = Sandbox::new("proptest");
        sandbox.plant();

        for arg in args {
            match sandbox.ws.read(&arg, 4096) {
                Ok(file) => prop_assert_eq!(file.bytes, INSIDE),
                Err(error) => prop_assert!(is_rejection(&error), "{error}"),
            }
        }
    }

    /// No generated argument may ever write outside the workspace. The
    /// outside sentinel must keep its exact bytes after every attempt.
    #[test]
    fn no_generated_argument_writes_outside(args in hostile_args()) {
        let sandbox = Sandbox::new("proptest-write");
        sandbox.plant();

        for arg in args {
            let _ = sandbox.ws.write(&arg, b"attempt");
            prop_assert_eq!(
                fs::read(sandbox.outside_path("outside.txt")).unwrap(),
                OUTSIDE
            );
            assert!(!sandbox.outside_path("attempted").exists());
        }
    }
}

#[test]
fn reads_and_writes_round_trip_inside_the_root() {
    let sandbox = Sandbox::new("roundtrip");
    sandbox.plant();

    let read = sandbox.ws.read("ok.txt", 4096).unwrap();
    assert_eq!(read.bytes, INSIDE);
    assert_eq!(read.size, INSIDE.len() as u64);
    assert!(!read.truncated);

    let nested = sandbox.ws.read("sub/nested.txt", 4096).unwrap();
    assert_eq!(nested.bytes, INSIDE);

    sandbox.ws.write("deep/dir/file.txt", b"created").unwrap();
    assert_eq!(
        sandbox.ws.read("deep/dir/file.txt", 4096).unwrap().bytes,
        b"created"
    );
}

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
fn root_is_resolved_once_and_canonical() {
    let sandbox = Sandbox::new("root");
    assert_eq!(sandbox.root(), fs::canonicalize(sandbox.root()).unwrap());
}

// --- Argument validation: NUL bytes, empty names, `.`, drive/UNC shapes. ---

#[test]
fn argument_validation_rejects_malformed_shapes() {
    let sandbox = Sandbox::new("validation");
    sandbox.plant();

    let cases = [
        "",
        "a\0b",
        ".",
        "./ok.txt",
        "sub/../.",
        "ok.txt/",
        "sub//nested.txt",
        "C:\\outside.txt",
        "C:/outside.txt",
        "\\\\srv\\share\\file",
    ];
    for arg in cases {
        let read = sandbox.ws.read(arg, 4096).expect_err("must reject");
        assert!(
            matches!(read, HarnessError::InvalidPath { .. }),
            "read({arg:?}) → {read:?}"
        );
        let write = sandbox.ws.write(arg, b"x").expect_err("must reject");
        assert!(
            matches!(write, HarnessError::InvalidPath { .. }),
            "write({arg:?}) → {write:?}"
        );
    }
}

#[test]
fn absolute_paths_and_dotdot_above_root_refuse() {
    let sandbox = Sandbox::new("escape");
    sandbox.plant();

    for arg in ["../outside.txt", "..", "sub/../../outside.txt"] {
        let error = sandbox.ws.read(arg, 4096).expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "read({arg:?}) → {error:?}"
        );
        let error = sandbox.ws.write(arg, b"escape").expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "write({arg:?}) → {error:?}"
        );
    }
    #[cfg(unix)]
    {
        let arg = "/etc/passwd";
        let error = sandbox.ws.read(arg, 4096).expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "read({arg:?}) → {error:?}"
        );
        let error = sandbox.ws.write(arg, b"escape").expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::PathOutsideRoot { .. }),
            "write({arg:?}) → {error:?}"
        );
    }
    assert_eq!(
        fs::read(sandbox.outside_path("outside.txt")).unwrap(),
        OUTSIDE
    );
}

// --- Symlink refusal: final component, chains, and parent components. ------

#[cfg(unix)]
#[test]
fn symlink_as_final_component_is_refused_not_followed() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("symlink-final");
    sandbox.plant();

    let outside = sandbox.outside_path("outside.txt");
    symlink(&outside, sandbox.root().join("link")).unwrap();

    let error = sandbox.ws.read("link", 4096).expect_err("link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );

    let error = sandbox
        .ws
        .write("link", b"overwrite-through-link")
        .expect_err("writing through a link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert_eq!(
        fs::read(&outside).unwrap(),
        OUTSIDE,
        "outside file untouched"
    );
}

#[cfg(unix)]
#[test]
fn symlink_chains_and_parent_components_are_refused() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("symlink-chain");
    sandbox.plant();

    let outside_dir = sandbox.outer.join("outside-dir");
    fs::create_dir_all(&outside_dir).unwrap();
    fs::write(outside_dir.join("secret.txt"), OUTSIDE).unwrap();

    // Chain: a → b → outside file.
    symlink(sandbox.root().join("b"), sandbox.root().join("a")).unwrap();
    symlink(
        sandbox.outside_path("outside.txt"),
        sandbox.root().join("b"),
    )
    .unwrap();

    for arg in ["a", "b"] {
        let error = sandbox.ws.read(arg, 4096).expect_err("chain must refuse");
        assert!(
            matches!(error, HarnessError::SymlinkRefused { .. }),
            "{error:?}"
        );
    }

    // Symlink in a parent component: dir_link/… never resolves outside.
    symlink(&outside_dir, sandbox.root().join("dir")).unwrap();
    let error = sandbox
        .ws
        .read("dir/secret.txt", 4096)
        .expect_err("parent-component link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    let error = sandbox
        .ws
        .write("dir/planted.txt", b"escape")
        .expect_err("write through a parent-component link must refuse");
    assert!(
        matches!(error, HarnessError::SymlinkRefused { .. }),
        "{error:?}"
    );
    assert!(
        !outside_dir.join("planted.txt").exists(),
        "nothing planted outside"
    );

    // Traversal starting from a name, then escaping above the root.
    let error = sandbox
        .ws
        .read("sub/../../../outside.txt", 4096)
        .expect_err("must refuse");
    assert!(
        matches!(error, HarnessError::PathOutsideRoot { .. }),
        "{error:?}"
    );
}

// --- TOCTOU: the final component is swapped between check and open. --------

/// Hammers `victim` with renames from a hostile thread while the reader
/// loops. `pass` performs one hostile pass; the reader must never observe
/// content from outside the workspace regardless of how the race lands.
/// Returns (ok reads, refusals observed, hostile passes completed).
fn race_final_component<F>(label: &str, iterations: usize, pass: F) -> (usize, usize, usize)
where
    F: Fn(&Path, &Path) + Send + 'static,
{
    let sandbox = Sandbox::new(label);
    sandbox.plant();
    let victim = sandbox.root().join("victim");
    fs::write(&victim, INSIDE).unwrap();
    let outside = sandbox.outside_path("outside.txt");

    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicUsize::new(0));
    let hostile_stop = Arc::clone(&stop);
    let hostile_swaps = Arc::clone(&swaps);
    let hostile = thread::spawn(move || {
        let stage = victim.with_extension("stage");
        while !hostile_stop.load(Ordering::Relaxed) {
            pass(&stage, &outside);
            if fs::rename(&stage, &victim).is_ok() {
                hostile_swaps.fetch_add(1, Ordering::Relaxed);
            }
        }
    });

    let mut ok = 0usize;
    let mut caught = 0usize;
    for _ in 0..iterations {
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

    (ok, caught, swaps.load(Ordering::Relaxed))
}

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

// --- Write race: the file is renamed away and back while it is read. -------

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

// --- Atomic writes and mode discipline. -----------------------------------

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

// --- Bound evasion. -------------------------------------------------------

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

// --- Hygiene deny-list (not a credentials control). -----------------------

#[test]
fn git_names_are_refused_as_hygiene_only() {
    let sandbox = Sandbox::new("git-hygiene");
    sandbox.plant();
    fs::create_dir_all(sandbox.root().join(".git")).unwrap();
    fs::write(sandbox.root().join(".git/config"), INSIDE).unwrap();

    for arg in [".git", ".git/config", ".GIT/config", "sub/.git/config"] {
        let error = sandbox.ws.read(arg, 4096).expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::DeniedName { .. }),
            "{arg:?} → {error:?}"
        );
        let error = sandbox.ws.write(arg, b"nope").expect_err("must refuse");
        assert!(
            matches!(error, HarnessError::DeniedName { .. }),
            "{arg:?} → {error:?}"
        );
    }
    // The manifest walk skips hygiene content silently.
    let manifest = saya_harness::workspace::manifest::build(&sandbox.ws, 100, 1 << 20).unwrap();
    assert!(manifest.iter().all(|entry| !entry.path.contains(".git")));
}

// --- Unicode / case-folding variants (macOS-gated). -----------------------

#[cfg(target_os = "macos")]
#[test]
fn nfd_and_case_aliases_resolve_inside_the_root_or_refuse() {
    use std::os::unix::fs::symlink;
    let sandbox = Sandbox::new("unicode-fold");
    sandbox.plant();

    // An NFD-named file must be reachable through its NFC spelling: the
    // kernel folds the lookup, and canonicalise + prefix keep it contained.
    let nfd = "r\u{e9}sum\u{e9}.txt"; // composed here; the FS stores NFD bytes
    let nfd_bytes: &str = "re\u{301}sume\u{301}.txt";
    sandbox.ws.write(nfd_bytes, INSIDE).unwrap();
    let aliased = sandbox.ws.read(nfd, 4096);
    assert!(
        aliased.as_ref().is_ok_and(|file| file.bytes == INSIDE) || {
            matches!(
                aliased,
                Err(HarnessError::NotFound { .. }) | Err(HarnessError::Io { .. })
            )
        },
        "NFC/NFD aliasing must stay contained: {aliased:?}"
    );

    // Case-insensitive lookup must not turn a case variant into a new escape:
    // a symlink reached through a folded spelling is still refused.
    symlink(
        sandbox.outside_path("outside.txt"),
        sandbox.root().join("Lin\u{413}k"),
    )
    .unwrap();
    for spelling in ["Lin\u{413}k", "li\u{413}k", "LIN\u{413}K"] {
        match sandbox.ws.read(spelling, 4096) {
            Ok(file) => assert_eq!(file.bytes, INSIDE, "folded read must stay inside"),
            Err(error) => {
                assert!(is_rejection(&error), "{spelling} → {error}");
                if matches!(error, HarnessError::SymlinkRefused { .. }) {
                    // The folded lookup landed on the link and was refused.
                }
            }
        }
    }
}

#[test]
#[cfg(all(not(target_os = "macos"), not(windows)))]
fn case_variants_do_not_alias_on_case_sensitive_filesystems() {
    let sandbox = Sandbox::new("case-sensitive");
    sandbox.plant();
    // An absent case variant is absence, not an escape: the typed `NotFound`
    // carries the workspace-relative name, exactly as the old `Io`-kind
    // `NotFound` did.
    assert!(matches!(
        sandbox.ws.read("OK.TXT", 4096),
        Err(HarnessError::NotFound { path }) if path == "OK.TXT"
    ));
}

/// Absence keeps the workspace-relative name in the typed error: no absolute
/// host path reaches the `NotFound` payload.
#[test]
fn a_missing_file_reports_its_workspace_relative_name() {
    let sandbox = Sandbox::new("not-found-path");
    sandbox.plant();
    assert!(matches!(
        sandbox.ws.read("absent.txt", 4096),
        Err(HarnessError::NotFound { path }) if path == "absent.txt"
    ));
    assert!(matches!(
        sandbox.ws.read("sub/absent.txt", 4096),
        Err(HarnessError::NotFound { path }) if path == "sub/absent.txt"
    ));
    assert!(matches!(
        sandbox.ws.read("no/such/dir.txt", 4096),
        Err(HarnessError::NotFound { path }) if path == "no/such/dir.txt"
    ));
}
