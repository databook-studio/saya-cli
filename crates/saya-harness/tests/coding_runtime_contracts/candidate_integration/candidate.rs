//! W08a candidate tests (authored, not executed): the content-only candidate
//! source digest over one verified managed checkout, and the CandidateIdentity
//! carried forward from the verified baseline.

use saya_harness::coding::integration::{
    CandidateError, CandidateIdentity, DEFAULT_DELTA_DEADLINE, DeltaError, candidate_source_digest,
};
use saya_harness::coding::snapshot::MAX_SNAPSHOT_FILE_BYTES;
use std::{fs, time::Duration};

use super::support_active::{Active, active};

#[tokio::test]
async fn candidate_digest_is_deterministic() {
    let a = active("w08-det-a", "checkout-w08-det-a").await;
    let b = active("w08-det-b", "checkout-w08-det-b").await;
    let first = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let second = candidate_source_digest(&b.root, &b.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(first, second);
    fs::write(b.root.join("extra.txt"), b"extra\n").unwrap();
    let changed = candidate_source_digest(&b.root, &b.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_ne!(first, changed);
    fs::remove_file(b.root.join("extra.txt")).unwrap();
    let restored = candidate_source_digest(&b.root, &b.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(first, restored);
}

#[tokio::test]
async fn candidate_digest_ignores_write_order() {
    let a = active("w08-order-a", "checkout-w08-order-a").await;
    let b = active("w08-order-b", "checkout-w08-order-b").await;
    fs::write(a.root.join("x.txt"), b"x bytes\n").unwrap();
    fs::write(a.root.join("y.txt"), b"y bytes\n").unwrap();
    fs::write(b.root.join("y.txt"), b"y bytes\n").unwrap();
    fs::write(b.root.join("x.txt"), b"x bytes\n").unwrap();
    let first = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    let second = candidate_source_digest(&b.root, &b.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(first, second);
}

#[tokio::test]
async fn candidate_digest_tracks_content_and_exec_bit() {
    let a = active("w08-sense", "checkout-w08-sense").await;
    let baseline = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();
    let after_content = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_ne!(baseline, after_content);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let path = a.root.join("a/one.txt");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).unwrap();
        let after_exec =
            candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
        assert_ne!(after_content, after_exec);
    }
}

#[tokio::test]
async fn candidate_digest_refuses_symlink_and_nested_git() {
    let a = active("w08-refuse", "checkout-w08-refuse").await;
    use std::os::unix::fs::symlink;

    symlink("tracked.txt", a.root.join("link")).unwrap();
    let err = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE)
        .err()
        .unwrap();
    assert!(matches!(err, CandidateError::Walk(DeltaError::Symlink)));
    fs::remove_file(a.root.join("link")).unwrap();
    fs::create_dir_all(a.root.join("sub/.git")).unwrap();
    let err = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE)
        .err()
        .unwrap();
    assert!(matches!(
        err,
        CandidateError::Walk(DeltaError::NestedRepository)
    ));
}

#[tokio::test]
async fn candidate_digest_rejects_oversize_instead_of_truncating() {
    let a = active("w08-big", "checkout-w08-big").await;
    let big = a.root.join("big.bin");
    fs::File::create(&big)
        .unwrap()
        .set_len(MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    let err = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE)
        .err()
        .unwrap();
    assert!(matches!(
        err,
        CandidateError::Walk(DeltaError::FileTooLarge)
    ));
}

#[tokio::test]
async fn candidate_digest_honours_cancellation_and_deadline() {
    let a = active("w08-cancel", "checkout-w08-cancel").await;
    a.token.cancel();
    let err = candidate_source_digest(&a.root, &a.token, DEFAULT_DELTA_DEADLINE)
        .err()
        .unwrap();
    assert!(matches!(err, CandidateError::Walk(DeltaError::Cancelled)));
    let b = active("w08-deadline", "checkout-w08-deadline").await;
    let err = candidate_source_digest(&b.root, &b.token, Duration::ZERO)
        .err()
        .unwrap();
    assert!(matches!(
        err,
        CandidateError::Walk(DeltaError::DeadlineExceeded)
    ));
}

#[tokio::test]
async fn candidate_identity_copies_the_verified_baseline_and_hides_paths() {
    let a: Active = active("w08-identity", "checkout-w08-identity").await;
    let identity = CandidateIdentity::from_verified(&a.fixture.job, &a.verified);
    assert_eq!(identity.job_id(), &a.fixture.job);
    assert_eq!(identity.checkout_id(), a.verified.checkout_id());
    assert_eq!(identity.snapshot_id(), a.verified.snapshot_id());
    assert_eq!(identity.snapshot_sha256(), a.verified.snapshot_sha256());
    assert_eq!(identity.plan_sha256(), a.verified.plan_sha256());
    let debug = format!("{identity:?}");
    assert!(debug.contains("checkout-w08-identity"));
    assert!(!debug.contains(a.root.to_string_lossy().as_ref()));
}
