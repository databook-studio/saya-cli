//! The checkpoint store's own contract: a rewind puts the working checkout back
//! to the checkpointed state exactly — including removing what the checkpoint
//! recorded as absent — the oldest checkpoint is the one the cap drops, and a
//! capture over a bound records nothing at all. The job-level and adapter
//! behaviour is in `job_tests.rs`.

use super::{CapturedPath, MAX_REWIND_CHECKPOINTS, RewindStore, apply, store_root};
use crate::commands::coding::CodingError;
use saya_harness::coding::integration::AssignedPaths;
use std::path::{Path, PathBuf};

/// One private temp directory, removed when the test ends.
struct Sandbox {
    root: PathBuf,
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Sandbox {
    fn new(case: &str) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("saya-rewind-{case}-{}-{stamp}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let root = root.canonicalize().unwrap();
        Self { root }
    }

    fn checkout(&self) -> PathBuf {
        let checkout = self.root.join("checkout");
        std::fs::create_dir_all(&checkout).unwrap();
        checkout
    }

    fn store(&self) -> RewindStore {
        RewindStore::open(&store_root(&self.root.join("records")))
    }

    fn write(&self, checkout: &Path, path: &str, content: &str) {
        let target = checkout.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, content).unwrap();
    }

    fn read(&self, checkout: &Path, path: &str) -> Option<String> {
        std::fs::read_to_string(checkout.join(path)).ok()
    }
}

/// Reads `paths` out of the checkout as a capture would, so a test records the
/// same state the job's own capture records.
fn capture_state(checkout: &Path, paths: &[&str]) -> Vec<CapturedPath> {
    let workspace = saya_harness::workspace::Workspace::open(checkout).unwrap();
    apply::read_paths(
        &workspace,
        &paths
            .iter()
            .map(|path| (*path).to_owned())
            .collect::<Vec<_>>(),
    )
    .unwrap()
}

/// The state after turn 1: `tracked.txt` holds "one" and `added.txt` does not
/// exist yet.
fn write_turn_one(sandbox: &Sandbox, checkout: &Path) {
    sandbox.write(checkout, "tracked.txt", "one");
}

/// The state after turn 2: `tracked.txt` holds "two" and `added.txt` exists.
fn write_turn_two(sandbox: &Sandbox, checkout: &Path) {
    sandbox.write(checkout, "tracked.txt", "two");
    sandbox.write(checkout, "added.txt", "created in turn two");
}

/// Two turns of edits, then a rewind to after turn 1: the checkout matches
/// turn 1 exactly — the tracked file back to its turn-one content, and the file
/// turn 2 created gone.
#[test]
fn rewind_restores_the_checkout_to_the_named_turn() {
    let sandbox = Sandbox::new("restore");
    let checkout = sandbox.checkout();
    let store = sandbox.store();

    write_turn_one(&sandbox, &checkout);
    store
        .capture(1, &capture_state(&checkout, &["tracked.txt"]), 1_000)
        .unwrap();
    write_turn_two(&sandbox, &checkout);
    store
        .capture(
            2,
            &capture_state(&checkout, &["tracked.txt", "added.txt"]),
            2_000,
        )
        .unwrap();
    assert_eq!(sandbox.read(&checkout, "tracked.txt").unwrap(), "two");

    let scope = AssignedPaths::whole_checkout();
    let report = apply::restore(
        &store,
        &checkout,
        &scope,
        1,
        &["tracked.txt".to_owned(), "added.txt".to_owned()],
        None,
        apply::Before {
            images: None,
            foreign: &[],
        },
    )
    .unwrap();

    assert_eq!(sandbox.read(&checkout, "tracked.txt").unwrap(), "one");
    assert_eq!(
        sandbox.read(&checkout, "added.txt"),
        None,
        "a file created after the checkpoint is removed"
    );
    assert_eq!(report.restored, 1);
    assert_eq!(report.removed, 1);
}

/// A file the checkpoint recorded as absent — the job deleted it — is restored
/// by removing whatever is there now.
#[test]
fn rewind_removes_a_file_the_checkpoint_recorded_as_absent() {
    let sandbox = Sandbox::new("absent");
    let checkout = sandbox.checkout();
    let store = sandbox.store();

    store
        .capture(
            1,
            &[CapturedPath {
                path: "gone.txt".into(),
                bytes: None,
            }],
            1_000,
        )
        .unwrap();
    sandbox.write(&checkout, "gone.txt", "written after the checkpoint");

    let scope = AssignedPaths::whole_checkout();
    let report = apply::restore(
        &store,
        &checkout,
        &scope,
        1,
        &[],
        None,
        apply::Before {
            images: None,
            foreign: &[],
        },
    )
    .unwrap();

    assert_eq!(sandbox.read(&checkout, "gone.txt"), None);
    assert_eq!(report.restored, 1);
}

/// The cap keeps the newest checkpoints and drops the oldest first.
#[test]
fn the_cap_drops_the_oldest_checkpoint_first() {
    let sandbox = Sandbox::new("cap");
    let checkout = sandbox.checkout();
    let store = sandbox.store();
    sandbox.write(&checkout, "tracked.txt", "zero");

    for turn in 1..=(MAX_REWIND_CHECKPOINTS as u32 + 2) {
        sandbox.write(&checkout, "tracked.txt", &format!("turn {turn}"));
        store
            .capture(
                turn,
                &capture_state(&checkout, &["tracked.txt"]),
                turn as i64,
            )
            .unwrap();
    }

    let turns: Vec<u32> = store.list().unwrap().iter().map(|c| c.turn).collect();
    assert_eq!(turns.len(), MAX_REWIND_CHECKPOINTS);
    assert_eq!(
        turns.first(),
        Some(&3),
        "the two oldest checkpoints are the ones dropped"
    );
    assert_eq!(turns.last(), Some(&(MAX_REWIND_CHECKPOINTS as u32 + 2)));
}

/// A checkpoint over a named bound records nothing at all: the listing never
/// shows a partial checkpoint, and the turn is reported as not checkpointed.
#[test]
fn a_capture_over_a_bound_records_nothing() {
    let sandbox = Sandbox::new("bound");
    let store = sandbox.store();
    let oversized = vec![CapturedPath {
        path: "big.bin".into(),
        bytes: Some(vec![0u8; super::MAX_REWIND_FILE_BYTES as usize + 1]),
    }];

    assert!(matches!(
        store.capture(1, &oversized, 1_000),
        Err(CodingError::RewindCaptureTooLarge)
    ));
    assert!(store.list().unwrap().is_empty());
    assert!(
        !store_root(&sandbox.root.join("records"))
            .join("turn-0001")
            .exists()
    );
}
