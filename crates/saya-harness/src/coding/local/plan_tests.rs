use std::{fs, path::PathBuf};

use saya_types::SnapshotId;

use super::identity::LocalRoot;
use super::plan::LocalBaseline;
use super::plan_error::LocalBaselineError;
use super::test_support::Temp;

/// One root folder and one state folder as siblings, so the root never
/// overlaps the state directory the identity check refuses.
fn fixture(label: &str) -> (Temp, PathBuf, PathBuf) {
    let temp = Temp::new(label);
    let root = temp.mkdir("root");
    let state = temp.mkdir("state");
    (temp, root, state)
}

fn marker(root: &LocalRoot, label: &str) -> LocalBaseline {
    LocalBaseline::edit_time(root, SnapshotId::parse(label).unwrap()).unwrap()
}

#[test]
fn edit_time_marker_names_the_root_and_holds_no_inventory() {
    let (_temp, root_path, state) = fixture("plan-marker");
    fs::write(root_path.join("a.txt"), b"hello").unwrap();

    let root = LocalRoot::open(&root_path, &state).unwrap();
    let baseline = marker(&root, "job-local-marker");

    assert_eq!(baseline.root_path(), root.path().to_str().unwrap());
    assert_eq!(baseline.repository_id(), root.id().as_repository_id());
    assert_eq!(baseline.snapshot_id().as_str(), "job-local-marker");
    let value: serde_json::Value = serde_json::from_slice(baseline.canonical_bytes()).unwrap();
    assert_eq!(value["mode"], "in_place");
    assert_eq!(value["schema_version"], 1);
    assert!(value.get("entries").is_none());
    assert!(value.get("omissions").is_none());
}

#[test]
fn edit_time_marker_round_trips_and_refuses_a_changed_digest() {
    let (_temp, root_path, state) = fixture("plan-marker-round-trip");
    let root = LocalRoot::open(&root_path, &state).unwrap();
    let baseline = marker(&root, "job-local-round-trip");

    let parsed =
        LocalBaseline::from_verified_bytes(baseline.canonical_bytes(), baseline.manifest_sha256())
            .unwrap();
    assert_eq!(parsed.canonical_bytes(), baseline.canonical_bytes());
    assert_eq!(parsed.manifest_sha256(), baseline.manifest_sha256());
    assert_eq!(parsed.snapshot_id(), baseline.snapshot_id());
    assert_eq!(parsed.repository_id(), baseline.repository_id());
    assert_eq!(parsed.root_path(), baseline.root_path());

    let mut tampered = baseline.canonical_bytes().to_vec();
    let last = tampered.len() - 2;
    tampered[last] ^= 0x01;
    assert!(matches!(
        LocalBaseline::from_verified_bytes(&tampered, baseline.manifest_sha256()),
        Err(LocalBaselineError::DigestMismatch)
    ));
}

#[test]
fn edit_time_marker_refuses_a_non_canonical_body() {
    let (_temp, root_path, state) = fixture("plan-marker-malformed");
    let root = LocalRoot::open(&root_path, &state).unwrap();
    let baseline = marker(&root, "job-local-malformed");
    let mut value: serde_json::Value = serde_json::from_slice(baseline.canonical_bytes()).unwrap();
    value["extra"] = serde_json::json!(true);
    let bytes = serde_json::to_vec(&value).unwrap();
    let digest = crate::coding::snapshot::digest(&bytes).unwrap();
    assert!(matches!(
        LocalBaseline::from_verified_bytes(&bytes, &digest),
        Err(LocalBaselineError::Malformed)
    ));
}
