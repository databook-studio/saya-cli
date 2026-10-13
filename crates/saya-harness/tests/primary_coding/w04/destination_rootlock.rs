//! Root coordination lock tests: `begin` and the stale-staging sweep
//! serialize on the root-level `.lock` file, so a sweep can never remove a
//! stage that exists but has not yet taken its own lock. Concurrency here
//! cannot prove the absence of a race — the test asserts the public
//! invariant, that every stage `begin` returns survives a zero-age sweep,
//! while sweeps run concurrently on another thread.

use std::{thread, time::Duration};

use saya_harness::coding::snapshot::SnapshotArtifacts;
use saya_types::SnapshotId;

use super::super::support::TempRoot;
use super::destination::{non_lock_entries, staging_entries};
use super::fixtures::artifacts_root;

const BEGINS: usize = 12;
const SWEEPS: usize = 64;

#[test]
fn root_lock_serializes_begin_and_sweep() {
    let temp = TempRoot::new("w04-dest-rootlock");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let mut live = Vec::new();

    thread::scope(|scope| {
        // A concurrent sweeper: under the root lock every pass must leave
        // every live stage exactly as it found it, so zero removals is the
        // only possible result across the whole run. No stage is dropped
        // while it runs: a stage being dropped has already unlinked its own
        // lock file, so a sweep that arrives mid-drop may legitimately
        // finish removing it, and that is not a live stage.
        scope.spawn(|| {
            for _ in 0..SWEEPS {
                assert_eq!(artifacts.sweep_stale_staging(Duration::ZERO).unwrap(), 0);
            }
        });
        for index in 0..BEGINS {
            let id = SnapshotId::parse(&format!("rootlock-{index}")).unwrap();
            let staged = artifacts.begin(&id).unwrap();
            // The returned stage must survive a zero-age sweep: its own
            // lock is held, and under the root lock no concurrent sweep
            // was ever able to see it between creation and lock
            // acquisition.
            assert_eq!(artifacts.sweep_stale_staging(Duration::ZERO).unwrap(), 0);
            live.push(staged);
            // Every stage begun so far is still in place.
            assert_eq!(staging_entries(&root).len(), live.len());
        }
    });
    // The sweeper has joined, so each drop is now sequential: it removes
    // exactly its own stage, and the root keeps only the persistent `.lock`
    // entry `open` leaves behind.
    while let Some(staged) = live.pop() {
        let before = staging_entries(&root).len();
        drop(staged);
        assert_eq!(staging_entries(&root).len(), before - 1);
    }
    assert_eq!(non_lock_entries(&root), 0);
}
