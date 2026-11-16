use std::{fs, time::Duration};

use saya_harness::coding::git::IgnoredPaths;
use saya_harness::coding::integration::{
    DEFAULT_DELTA_DEADLINE, DeltaError, DeltaKind, NotIncludedReason, compute_delta,
};
use saya_harness::coding::snapshot::MAX_SNAPSHOT_FILE_BYTES;

use super::worktree_delta::active;

#[cfg(unix)]
#[tokio::test]
async fn relative_symlink_is_a_delta_while_unsafe_symlinks_and_nested_git_are_not_included() {
    let a = active("w07-edges-links", "checkout-w07-edges-links").await;
    std::os::unix::fs::symlink("tracked.txt", a.root.join("link")).unwrap();
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    let link = delta
        .entries()
        .iter()
        .find(|entry| entry.path() == "link")
        .expect("the relative symlink is a live delta");
    assert_eq!(link.kind(), DeltaKind::Added);
    assert_eq!(link.bytes(), "tracked.txt".len() as u64);
    fs::remove_file(a.root.join("link")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", a.root.join("link")).unwrap();
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("an unsafe symlink is reported, not fatal");
    assert!(delta.changes_truncated());
    assert!(
        delta.not_included().iter().any(|item| {
            item.path() == "link" && item.reason() == NotIncludedReason::UnsafePath
        })
    );
    fs::remove_file(a.root.join("link")).unwrap();
    fs::create_dir_all(a.root.join("sub/.git")).unwrap();
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("a nested repository is reported, not fatal");
    assert!(delta.changes_truncated());
    assert!(delta.not_included().iter().any(|item| {
        item.path() == "sub/.git" && item.reason() == NotIncludedReason::NestedRepository
    }));
}

#[tokio::test]
async fn verified_candidate_delta_reports_a_root_git_directory_not_included() {
    let a = active("w07-root-git-dir", "checkout-w07-root-git-dir").await;
    let pointer = a.root.join(".git");
    let pointer_bytes = fs::read(&pointer).unwrap();
    fs::remove_file(&pointer).unwrap();
    fs::create_dir(&pointer).unwrap();

    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("a root Git directory is reported, not fatal");

    fs::remove_dir(&pointer).unwrap();
    fs::write(pointer, pointer_bytes).unwrap();
    assert!(delta.changes_truncated());
    assert!(delta.not_included().iter().any(|item| {
        item.path() == ".git" && item.reason() == NotIncludedReason::NestedRepository
    }));
}

#[tokio::test]
async fn oversize_file_is_truncated_instead_of_rejecting() {
    let a = active("w07-edges-oversize", "checkout-w07-edges-oversize").await;
    fs::File::create(a.root.join("big.bin"))
        .unwrap()
        .set_len(MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    )
    .expect("an oversize file is reported, not fatal");
    assert!(delta.changes_truncated());
    assert!(
        delta.not_included().iter().any(|item| {
            item.path() == "big.bin" && item.reason() == NotIncludedReason::Oversize
        })
    );
}

#[tokio::test]
async fn cancelled_computations_record_no_delta() {
    let a = active("w07-edges-cancel", "checkout-w07-edges-cancel").await;
    a.token.cancel();
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    );
    assert!(matches!(delta, Err(DeltaError::Cancelled)));
}

#[tokio::test]
async fn expired_deadline_truncates() {
    let a = active("w07-edges-expired", "checkout-w07-edges-expired").await;
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        Duration::ZERO,
    )
    .expect("a spent deadline truncates the delta");
    assert!(delta.changes_truncated());
}

#[tokio::test]
async fn mismatched_snapshot_is_refused() {
    let first = active("w07-edges-mismatch", "checkout-w07-edges-mismatch").await;
    let second = active("w07-other", "checkout-w07-other").await;
    let delta = compute_delta(
        &first.verified,
        &second.snapshot,
        &IgnoredPaths::none(),
        &first.token,
        DEFAULT_DELTA_DEADLINE,
    );
    assert!(matches!(delta, Err(DeltaError::SnapshotMismatch)));
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn case_only_difference_is_a_collision() {
    let a = active("w07-edges-casefold", "checkout-w07-edges-casefold").await;
    fs::write(a.root.join("Tracked.txt"), b"case twin\n").unwrap();
    let delta = compute_delta(
        &a.verified,
        &a.snapshot,
        &IgnoredPaths::none(),
        &a.token,
        DEFAULT_DELTA_DEADLINE,
    );
    assert!(matches!(delta, Err(DeltaError::CaseFoldCollision)));
}
