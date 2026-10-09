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

#[path = "workspace_containment/import_reads.rs"]
mod import_reads;

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

#[path = "workspace_containment/path_validation.rs"]
mod path_validation;

#[path = "workspace_containment/file_bounds.rs"]
mod file_bounds;

// --- Argument validation: NUL bytes, empty names, `.`, drive/UNC shapes. ---

// --- Symlink refusal: final component, chains, and parent components. ------

#[path = "workspace_containment/symlink_refusals.rs"]
mod symlink_refusals;

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

#[path = "workspace_containment/race_safety.rs"]
mod race_safety;

// --- Write race: the file is renamed away and back while it is read. -------

// --- Atomic writes and mode discipline. -----------------------------------

// --- Bound evasion. -------------------------------------------------------

// --- Hygiene deny-list (not a credentials control). -----------------------

#[path = "workspace_containment/hygiene_platform.rs"]
mod hygiene_platform;

// --- Unicode / case-folding variants (macOS-gated). -----------------------
