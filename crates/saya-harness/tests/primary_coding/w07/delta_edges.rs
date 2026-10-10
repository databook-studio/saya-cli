use std::{fs, time::Duration};

use saya_harness::coding::integration::{DEFAULT_DELTA_DEADLINE, DeltaError, compute_delta};
use saya_harness::coding::snapshot::MAX_SNAPSHOT_FILE_BYTES;

use super::delta::active;

#[cfg(unix)]
#[tokio::test]
async fn symlink_rejects_then_nested_git_rejects() {
    let a = active("w07-edges-links", "checkout-w07-edges-links").await;
    std::os::unix::fs::symlink("tracked.txt", a.root.join("link")).unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta, Err(DeltaError::Symlink)));
    fs::remove_file(a.root.join("link")).unwrap();
    fs::create_dir_all(a.root.join("sub/.git")).unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta, Err(DeltaError::NestedRepository)));
}

#[tokio::test]
async fn oversize_file_rejects_instead_of_truncating() {
    let a = active("w07-edges-oversize", "checkout-w07-edges-oversize").await;
    fs::File::create(a.root.join("big.bin"))
        .unwrap()
        .set_len(MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta, Err(DeltaError::FileTooLarge)));
}

#[tokio::test]
async fn cancelled_and_expired_computations_record_no_delta() {
    let a = active("w07-edges-cancel", "checkout-w07-edges-cancel").await;
    a.token.cancel();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta, Err(DeltaError::Cancelled)));
}

#[tokio::test]
async fn expired_deadline_rejects() {
    let a = active("w07-edges-expired", "checkout-w07-edges-expired").await;
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, Duration::ZERO);
    assert!(matches!(delta, Err(DeltaError::DeadlineExceeded)));
}

#[tokio::test]
async fn mismatched_snapshot_is_refused() {
    let first = active("w07-edges-mismatch", "checkout-w07-edges-mismatch").await;
    let second = active("w07-other", "checkout-w07-other").await;
    let delta = compute_delta(
        &first.verified,
        &second.snapshot,
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
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE);
    assert!(matches!(delta, Err(DeltaError::CaseFoldCollision)));
}
