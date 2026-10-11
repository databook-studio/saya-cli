//! W07b delta tests (authored, not executed): the real checkout delta over
//! one verified managed checkout, classified against its published baseline.

use saya_agent::CancellationToken;
use saya_harness::coding::integration::{DEFAULT_DELTA_DEADLINE, DeltaKind, compute_delta};
use saya_harness::coding::snapshot::CapturedSnapshot;
use saya_harness::coding::worktree::{
    OwnershipInput, VerifiedCheckout, create_managed_checkout, verify_owned_checkout,
};
use saya_types::ManagedCheckoutId;
use std::{fs, path::PathBuf};

use super::create_support::CreateCheckoutFixture;

/// One real Active managed checkout, verified, with its published baseline.
pub(super) struct Active {
    pub(super) verified: VerifiedCheckout,
    pub(super) snapshot: CapturedSnapshot,
    pub(super) root: PathBuf,
    pub(super) token: CancellationToken,
}

pub(super) async fn active(label: &str, id: &str) -> Active {
    let fixture = CreateCheckoutFixture::new(label).await;
    let checkout = ManagedCheckoutId::parse(id).unwrap();
    let token = CancellationToken::new();
    create_managed_checkout(&fixture.input(&checkout), &token)
        .await
        .unwrap();
    let input = OwnershipInput {
        store: &fixture.store,
        service: &fixture.service,
        repository: &fixture.repository,
        artifacts: &fixture.artifacts,
        job_id: &fixture.job,
        checkout_id: &checkout,
    };
    let verified = verify_owned_checkout(&input, &token).await.unwrap();
    let snapshot = fixture
        .artifacts
        .read_snapshot(verified.snapshot_id())
        .unwrap();
    let root = PathBuf::from(fixture.service.managed_path(&checkout).unwrap().as_str());
    Active {
        verified,
        snapshot,
        root,
        token,
    }
}

#[tokio::test]
async fn untouched_checkout_has_an_empty_delta() {
    let a = active("w07-delta-clean", "checkout-w07-clean").await;
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert!(delta.is_empty());
}

#[tokio::test]
async fn added_hidden_and_omitted_files_count() {
    let a = active("w07-delta-added", "checkout-w07-added").await;
    fs::write(a.root.join(".hidden-note"), b"hidden\n").unwrap();
    fs::create_dir_all(a.root.join("notes")).unwrap();
    fs::write(a.root.join("notes/new.txt"), b"new bytes\n").unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(delta.entries().len(), 2);
    assert_eq!(delta.entries()[0].path(), ".hidden-note");
    assert_eq!(delta.entries()[0].kind(), DeltaKind::Added);
    assert_eq!(delta.entries()[0].bytes(), 7);
    assert!(delta.entries()[0].content_sha256().is_some());
    assert_eq!(delta.entries()[1].path(), "notes/new.txt");
    assert_eq!(delta.entries()[1].kind(), DeltaKind::Added);
    assert_eq!(delta.entries()[1].bytes(), 10);
    assert!(delta.entries()[1].content_sha256().is_some());
}

#[tokio::test]
async fn modified_and_deleted_classify_with_bytes() {
    let a = active("w07-delta-classify", "checkout-w07-classify").await;
    fs::write(a.root.join("tracked.txt"), b"changed\n").unwrap();
    fs::remove_file(a.root.join("a/one.txt")).unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(delta.entries().len(), 2);
    assert_eq!(delta.entries()[0].path(), "a/one.txt");
    assert_eq!(delta.entries()[0].kind(), DeltaKind::Deleted);
    assert_eq!(delta.entries()[0].bytes(), 0);
    assert!(delta.entries()[0].content_sha256().is_none());
    assert_eq!(delta.entries()[1].path(), "tracked.txt");
    assert_eq!(delta.entries()[1].kind(), DeltaKind::Modified);
    assert_eq!(delta.entries()[1].bytes(), 8);
    assert!(delta.entries()[1].content_sha256().is_some());
}

#[cfg(unix)]
#[tokio::test]
async fn exec_bit_flip_is_a_modification() {
    use std::os::unix::fs::PermissionsExt;

    let a = active("w07-delta-exec", "checkout-w07-exec").await;
    let path = a.root.join("a/one.txt");
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&path, permissions).unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(delta.entries().len(), 1);
    assert_eq!(delta.entries()[0].path(), "a/one.txt");
    assert_eq!(delta.entries()[0].kind(), DeltaKind::Modified);
}

#[tokio::test]
async fn delta_debug_carries_no_path() {
    let a = active("w07-delta-debug", "checkout-w07-debug").await;
    fs::create_dir_all(a.root.join("secret-dir")).unwrap();
    fs::write(a.root.join("secret-dir/x.txt"), b"secret bytes\n").unwrap();
    let delta = compute_delta(&a.verified, &a.snapshot, &a.token, DEFAULT_DELTA_DEADLINE).unwrap();
    assert_eq!(delta.entries().len(), 1);
    assert!(!format!("{delta:?}").contains("secret-dir"));
    assert!(!format!("{:?}", delta.entries()[0]).contains("secret-dir"));
}
