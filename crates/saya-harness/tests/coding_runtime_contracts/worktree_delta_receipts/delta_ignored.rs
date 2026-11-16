//! Parity: the managed delta follows the checkout's own Git ignore rules,
//! reports special paths instead of failing, and truncates a bound trip
//! instead of rejecting the whole review.

use std::{fs, time::Duration};

use saya_harness::coding::git::IgnoredPaths;
use saya_harness::coding::integration::{
    AcceptedDelta, AssignedPaths, DEFAULT_DELTA_DEADLINE, DeltaKind, EvidenceBinding,
    NotIncludedReason, candidate_source_digest, compute_delta, evaluate_receipt,
};
use saya_harness::coding::snapshot::MAX_SNAPSHOT_FILE_BYTES;

use super::worktree_delta::{Active, active};

/// The checkout's own Git ignore rules, read through the Git service.
async fn ignored(a: &Active) -> IgnoredPaths {
    a.fixture
        .service
        .ignored_paths(&a.root, &a.token, DEFAULT_DELTA_DEADLINE)
        .await
        .expect("the checkout's ignore rules are read")
}

/// A job that builds a large ignored `target/` tree (including a nested
/// ignored `.git`) still reviews and applies its real edits, and the source
/// digest covers the same included set.
#[tokio::test]
async fn ignored_build_output_is_skipped_and_real_edits_survive() {
    let a = active("w07-delta-ignored", "checkout-w07-delta-ignored").await;
    fs::write(a.root.join(".gitignore"), b"target/\n").unwrap();
    let target = a.root.join("target");
    fs::create_dir_all(&target).unwrap();
    for index in 0..60_000 {
        fs::write(target.join(format!("object-{index}")), b"").unwrap();
    }
    fs::create_dir_all(target.join(".git")).unwrap();
    fs::write(target.join(".git/HEAD"), b"ref: refs/heads/nested\n").unwrap();
    fs::create_dir_all(a.root.join("notes")).unwrap();
    fs::write(a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();
    fs::remove_file(a.root.join("a/one.txt")).unwrap();

    let ignored = ignored(&a).await;
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &ignored,
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("ignored build output must not fail the delta");
    let paths: Vec<_> = delta.entries().iter().map(|entry| entry.path()).collect();
    assert_eq!(
        paths,
        vec![".gitignore", "a/one.txt", "notes/new.txt", "tracked.txt"]
    );
    assert_eq!(delta.entries()[1].kind(), DeltaKind::Deleted);
    assert_eq!(delta.entries()[3].kind(), DeltaKind::Modified);
    assert!(!delta.changes_truncated());
    assert!(delta.not_included().is_empty());

    let binding = EvidenceBinding::bind(&a.verified, &ignored, &a.token, DEFAULT_DELTA_DEADLINE)
        .expect("the source digest must skip the same ignored paths");
    // The digest covers exactly the included set: removing the ignored tree
    // from the checkout leaves the same digest.
    fs::remove_dir_all(&target).unwrap();
    let without_target = candidate_source_digest(
        &a.root,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert_eq!(binding.source_sha256(), &without_target);

    // Apply-back preflight sees the same delta: no path is silently omitted.
    let receipt = evaluate_receipt(
        &a.verified,
        &delta,
        &AssignedPaths::whole_checkout(),
        64 * 1024 * 1024,
    );
    AcceptedDelta::new(&a.verified, &receipt, &delta).expect("the delta preflights for apply");
}

#[cfg(unix)]
#[tokio::test]
async fn a_non_ignored_special_file_is_reported_not_included() {
    let a = active("w07-delta-special", "checkout-w07-special").await;
    std::os::unix::fs::symlink("/etc/passwd", a.root.join("link")).unwrap();
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();

    let ignored = ignored(&a).await;
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &ignored,
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("a special file is reported, not fatal");
    assert!(!delta.entries().iter().any(|entry| entry.path() == "link"));
    assert!(delta.changes_truncated());
    assert!(
        delta.not_included().iter().any(|item| {
            item.path() == "link" && item.reason() == NotIncludedReason::UnsafePath
        })
    );
}

#[tokio::test]
async fn an_oversize_file_is_truncated_and_reported() {
    let a = active("w07-delta-oversize", "checkout-w07-oversize").await;
    fs::File::create(a.root.join("big.bin"))
        .unwrap()
        .set_len(MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();

    let ignored = ignored(&a).await;
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &ignored,
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("an oversize file is reported, not fatal");
    assert!(
        !delta
            .entries()
            .iter()
            .any(|entry| entry.path() == "big.bin")
    );
    assert_eq!(delta.entries().len(), 1);
    assert_eq!(delta.entries()[0].path(), "tracked.txt");
    assert!(delta.changes_truncated());
    assert!(
        delta.not_included().iter().any(|item| {
            item.path() == "big.bin" && item.reason() == NotIncludedReason::Oversize
        })
    );
}

#[tokio::test]
async fn an_expired_deadline_truncates_instead_of_rejecting() {
    let a = active("w07-delta-expired", "checkout-w07-expired").await;
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();

    let ignored = ignored(&a).await;
    let delta = compute_delta(&a.verified, &a.snapshot, &ignored, &a.token, Duration::ZERO)
        .expect("a spent deadline truncates the delta");
    assert!(delta.is_empty());
    assert!(delta.changes_truncated());
}
