//! The intermediate-component TOCTOU sentinel. The containment battery's
//! swap loops attack the *final* component, which the no-follow open guards;
//! this test attacks the seam's other half: a concurrent swapper replacing an
//! *intermediate* directory with a link to a directory outside the root while
//! contained reads, writes, and listings run. Whatever the walk resolved must
//! be what is served, written, and listed — never what the path names after
//! the swap.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::Duration,
};

use saya_harness::{HarnessError, workspace::Workspace};

const INSIDE: &[u8] = b"inside-sentinel";
const OUTSIDE: &[u8] = b"outside-sentinel-MUST-NOT-LEAK";
const MIRROR: &[u8] = b"swapper-mirror";
const TARGET: &str = "sub/dir/nested.txt";

struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-toctou-mid-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox root");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn root(&self) -> &Path {
        self.ws.root()
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

/// Swaps `sub` between the real directory (parked at `sub.stage`) and a
/// symlink to `outside_dir`, and watches the real directory for a contained
/// writer's temp file. A temp present means that writer's walk and
/// canonicalise have already passed; between its temp creation and its
/// by-path rename sits a flush of milliseconds. Mirroring the temp's name
/// outside and holding the swapped link in place across that window makes the
/// writer's rename resolve through the swapped intermediate — the seam this
/// test exists to expose. The self-healing dance keeps the sentinel directory
/// at exactly one of the two slots, so clearing whatever else shows up at
/// `sub` (a contained write recreated it) never loses the sentinel content.
fn swap_sub(root: &Path, outside_dir: &Path, stop: &Arc<AtomicBool>, swaps: &Arc<AtomicUsize>) {
    let real = root.join("sub");
    let stage = root.join("sub.stage");
    let bait = outside_dir.join("dir");
    while !stop.load(Ordering::Relaxed) {
        // Detect a writer mid-flight: its temp lives inside the real
        // directory already, the walk and canonicalise behind it.
        let temps: Vec<String> = match fs::read_dir(real.join("dir")) {
            Ok(entries) => entries
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| name.starts_with(".saya-tmp-"))
                .collect(),
            Err(_) => Vec::new(),
        };
        if !temps.is_empty() {
            // Mirror the temp names into the bait directory, then swap the
            // link in and hold it across the writer's flush-and-rename.
            for name in &temps {
                let _ = fs::write(bait.join(name), MIRROR);
            }
            if fs::rename(&real, &stage).is_ok() {
                let _ = std::os::unix::fs::symlink(outside_dir, &real);
                thread::sleep(Duration::from_millis(20));
                let _ = fs::remove_file(&real);
                let _ = fs::rename(&stage, &real);
                swaps.fetch_add(1, Ordering::Relaxed);
            }
            for name in &temps {
                let _ = fs::remove_file(bait.join(name));
            }
            continue;
        }
        // Between detections: the continuous swap dance, so reads and
        // listings also sample every state of the swapped intermediate.
        if fs::rename(&real, &stage).is_ok() {
            let _ = std::os::unix::fs::symlink(outside_dir, &real);
            let _ = fs::remove_file(&real);
            if fs::rename(&stage, &real).is_err() {
                let _ = fs::remove_dir_all(&real);
                let _ = fs::remove_file(&real);
                let _ = fs::rename(&stage, &real);
            }
            swaps.fetch_add(1, Ordering::Relaxed);
        } else {
            let _ = fs::remove_dir_all(&real);
            let _ = fs::remove_file(&real);
            let _ = fs::rename(&stage, &real);
        }
    }
}

#[cfg(unix)]
#[test]
fn intermediate_directory_swap_never_crosses_the_root() {
    let sandbox = Sandbox::new("swap");
    // Inside: the walk target, two components deep, with sentinel content.
    sandbox.ws.write(TARGET, INSIDE).unwrap();
    // Outside: the bait the swap serves — mirrored under the swapped
    // component, same names, different bytes — plus a name no contained call
    // may ever create or list.
    let outside_dir = sandbox.outer.join("outside-dir");
    fs::create_dir_all(outside_dir.join("dir")).unwrap();
    fs::write(outside_dir.join("dir/nested.txt"), OUTSIDE).unwrap();
    fs::write(outside_dir.join("bait-listed.txt"), OUTSIDE).unwrap();

    let stop = Arc::new(AtomicBool::new(false));
    let swaps = Arc::new(AtomicUsize::new(0));
    let hostile = {
        let stop = Arc::clone(&stop);
        let swaps = Arc::clone(&swaps);
        let root = sandbox.root().to_path_buf();
        let outside_dir = outside_dir.clone();
        thread::spawn(move || swap_sub(&root, &outside_dir, &stop, &swaps))
    };

    // Writes carry an fsync per attempt and are paced so the loop stays
    // dominated by the race itself, not by flush latency.
    let mut ok_reads = 0usize;
    for round in 0..20_000 {
        match sandbox.ws.read(TARGET, 4096) {
            Ok(file) => {
                ok_reads += 1;
                assert_eq!(file.bytes, INSIDE, "outside content must never be served");
            }
            Err(error) => assert!(is_rejection(&error), "read {round}: {error}"),
        }
        match sandbox.ws.list("sub", 100) {
            Ok(entries) => {
                for entry in &entries {
                    assert_ne!(entry.name, "bait-listed.txt", "outside listing leaked");
                }
            }
            Err(error) => assert!(is_rejection(&error), "list {round}: {error}"),
        }
        if round % 100 == 0 {
            match sandbox.ws.write("sub/dir/out.txt", INSIDE) {
                Ok(()) => {}
                Err(error) => assert!(is_rejection(&error), "write {round}: {error}"),
            }
            // Nothing the layer did may have landed outside the root,
            // whatever the race resolved in between: no new names there,
            // bait untouched. The swapper's own transient mirrors are
            // `.saya-tmp-*` and are cleaned up by it; anything else is a
            // contained write that escaped.
            let strays: Vec<String> = fs::read_dir(outside_dir.join("dir"))
                .unwrap()
                .filter_map(|entry| entry.ok())
                .filter_map(|entry| entry.file_name().into_string().ok())
                .filter(|name| name != "nested.txt" && !name.starts_with(".saya-tmp-"))
                .collect();
            assert!(
                strays.is_empty(),
                "a contained write landed outside the root: {strays:?}"
            );
            assert_eq!(
                fs::read(outside_dir.join("dir/nested.txt")).unwrap(),
                OUTSIDE,
                "outside bait untouched"
            );
        }
    }
    stop.store(true, Ordering::Relaxed);
    hostile.join().unwrap();
    assert!(
        swaps.load(Ordering::Relaxed) > 0,
        "hostile thread must have raced at all"
    );
    assert!(
        ok_reads > 10,
        "the swapper must leave the real directory in place often enough \
         for walks to pass through it (ok reads: {ok_reads})"
    );
}
