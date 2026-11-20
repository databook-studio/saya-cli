//! What one delta does with a checkout that moves under it: a path that leaves
//! the tree between being listed and being read is not a refusal, but a tree
//! that never holds still still is — and the walk is bounded there.
//!
//! The walk's test seam removes (or recreates) a listed path at exactly that
//! step, so neither of these cases depends on machine load or on a real
//! concurrent writer.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use saya_agent::CancellationToken;

use super::seam::{self, Step};
use crate::coding::git::IgnoredPaths;
use crate::coding::integration::{
    CandidateError, DEFAULT_DELTA_DEADLINE, DeltaError, candidate_source_digest,
};

#[test]
fn a_walk_that_loses_a_listed_entry_settles_on_the_tree_that_remains() {
    let root = temp_root("settles");
    write(&root, "keep.txt", "keep\n");
    write(&root, "canary.txt", "canary\n");
    let canary = root.join("canary.txt");
    let mut removed = false;
    // The canary leaves the tree between the listing and the read, the way a
    // concurrent cleanup does. The tree that remains is a tree the walk can
    // fingerprint, so that is the generation the walk must report.
    seam::arm(move |stage, relative| {
        if let Step::Listed = stage
            && relative == "canary.txt"
            && !removed
        {
            removed = true;
            std::fs::remove_file(&canary).expect("the test removes its canary");
        }
    });
    let generation = candidate_source_digest(
        &root,
        &IgnoredPaths::none(),
        &CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    );
    seam::disarm();
    let settled = candidate_source_digest(
        &root,
        &IgnoredPaths::none(),
        &CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("the settled tree fingerprints");
    assert_eq!(
        generation.expect("a listed entry that vanished is walked again, not refused"),
        settled,
        "the generation is the one of the tree that remains"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_walk_that_keeps_losing_a_listed_entry_gives_up_after_three_attempts() {
    let root = temp_root("gives-up");
    write(&root, "keep.txt", "keep\n");
    let canary = root.join("canary.txt");
    let attempt = Arc::new(AtomicUsize::new(0));
    let counting = Arc::clone(&attempt);
    let recreate = canary.clone();
    // Every attempt finds a canary, and every attempt loses it before reading
    // it: a checkout that never holds still enough to be fingerprinted.
    seam::arm(move |stage, relative| match stage {
        Step::WalkStarted => {
            counting.fetch_add(1, Ordering::AcqRel);
            std::fs::write(&recreate, "canary\n").expect("the test recreates its canary");
        }
        Step::Listed if relative == "canary.txt" => {
            std::fs::remove_file(&canary).expect("the test removes its canary");
        }
        Step::Listed => {}
    });
    let outcome = candidate_source_digest(
        &root,
        &IgnoredPaths::none(),
        &CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    );
    seam::disarm();
    let error = outcome.expect_err("a tree that never holds still is not fingerprinted");
    assert!(
        matches!(error, CandidateError::Walk(DeltaError::Vanished)),
        "the walk names the vanished entry rather than an unreadable checkout: {error:?}"
    );
    assert_eq!(
        attempt.load(Ordering::Acquire),
        3,
        "the walk stops at three attempts instead of looping: {error}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A fresh root under `./target`, so a test never writes outside the workspace.
fn temp_root(label: &str) -> PathBuf {
    static NONCE: AtomicU64 = AtomicU64::new(0);
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!(
            "tmp-delta-walk-{label}-{}-{}",
            std::process::id(),
            NONCE.fetch_add(1, Ordering::AcqRel)
        ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("the test root is created");
    path
}

fn write(root: &Path, name: &str, content: &str) {
    std::fs::write(root.join(name), content).expect("the test writes its file");
}
