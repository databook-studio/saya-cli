#![cfg(unix)]
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "w05/create_support.rs"]
mod create_support;
#[allow(clippy::duplicate_mod)] // intentional: this test group links its own private copy of the shared fixture (own statics)
#[allow(dead_code)]
#[path = "w02_support.rs"]
mod support;
use saya_harness::coding::snapshot::{
    MAX_SNAPSHOT_MANIFEST_BYTES, SnapshotError, SnapshotManifest,
};
use saya_types::Sha256Digest;
use sha2::{Digest, Sha256};

#[tokio::test]
async fn restore_preserves_original_plan_and_refuses_malformed_or_unbound_bytes() {
    let fixture = create_support::CreateCheckoutFixture::new("restore-plan").await;
    let original = fixture.plan.canonical_bytes();
    let expected = fixture.plan.manifest_sha256();
    std::fs::write(fixture.repo_root.join("tracked.txt"), "later user edit").unwrap();
    let restored = SnapshotManifest::from_verified_bytes(original, expected).unwrap();
    assert_eq!(restored.canonical_bytes(), original);
    assert_eq!(restored.snapshot_id(), fixture.plan.snapshot_id());
    assert_eq!(
        std::fs::read_to_string(fixture.repo_root.join("tracked.txt")).unwrap(),
        "later user edit"
    );
    let wrong = Sha256Digest::parse(&"0".repeat(64)).unwrap();
    assert!(matches!(
        SnapshotManifest::from_verified_bytes(original, &wrong),
        Err(SnapshotError::ManifestDigestMismatch)
    ));
    assert!(matches!(
        SnapshotManifest::from_verified_bytes(
            &vec![b' '; MAX_SNAPSHOT_MANIFEST_BYTES + 1],
            expected
        ),
        Err(SnapshotError::TooLarge)
    ));
    let mut tampered: serde_json::Value = serde_json::from_slice(original).unwrap();
    tampered["entries"][0]["path"] = "../outside".into();
    let bytes = serde_json::to_vec(&tampered).unwrap();
    let digest = Sha256Digest::parse(&format!("{:x}", Sha256::digest(&bytes))).unwrap();
    assert!(SnapshotManifest::from_verified_bytes(&bytes, &digest).is_err());
}
