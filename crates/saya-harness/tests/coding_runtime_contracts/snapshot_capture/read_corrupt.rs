//! Corrupt-published-snapshot refusal tests (authored, not executed): a
//! manifest byte that no longer matches COMPLETE refuses with the digest
//! mismatch, while COMPLETE-consistent rewrites — an unknown field, a nested
//! extra field inside an entry's source, a newer schema version, traversal
//! and absolute entry paths, an id that differs from the directory, an
//! over-bound declared size, and duplicate paths —
//! refuse with the typed malformed error, and a symlinked manifest,
//! COMPLETE, or snapshot directory refuses without following. Public API
//! only, real temporary repositories.

use std::{fs, os::unix::fs::symlink, path::Path};

use saya_harness::coding::snapshot::SnapshotError;
use saya_types::SnapshotId;

use super::fixtures::{artifacts_root, digest};
use super::read::{publish_current, publish_head};

/// Rewrites the published manifest.json with `edit` applied to its parsed
/// JSON body and refreshes COMPLETE to the new bytes' digest, so only the
/// manifest-content checks can refuse the read.
pub(super) fn rewrite_manifest(
    manifest_path: &Path,
    complete_path: &Path,
    original: &[u8],
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let mut value: serde_json::Value = serde_json::from_slice(original).unwrap();
    edit(&mut value);
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(manifest_path, &bytes).unwrap();
    fs::write(complete_path, digest(&bytes).as_str()).unwrap();
}

/// Asserts the setup premise every entries rewrite relies on: the captured
/// manifest carries at least one entry.
pub(super) fn assert_has_entries(original: &[u8]) {
    let value: serde_json::Value = serde_json::from_slice(original).unwrap();
    assert!(!value["entries"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn flipped_manifest_byte_refuses_with_digest_mismatch() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-flip").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let original = fs::read(&manifest_path).unwrap();
    // One byte flips inside a path string: same length, still valid JSON,
    // and COMPLETE still names the original bytes, so only the manifest
    // digest check can refuse the read.
    let mut flipped = original.clone();
    let marker = b"tracked.txt";
    let at = flipped
        .windows(marker.len())
        .position(|window| window == marker)
        .expect("the fixture path appears in the manifest");
    flipped[at] = b'c';
    fs::write(&manifest_path, &flipped).unwrap();
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("a flipped manifest byte refuses the read");
    assert!(matches!(refused, SnapshotError::ManifestDigestMismatch));
}

#[tokio::test]
async fn unknown_field_and_bad_schema_version_refuse() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-schema").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    // An extra top-level field, with COMPLETE refreshed to the new bytes:
    // only the schema check can refuse the read.
    rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
        value["unknown_extra"] = serde_json::json!("unexpected");
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("an unknown field refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
    // A schema_version the reader does not know, again with COMPLETE
    // refreshed: only the schema check can refuse the read.
    rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
        value["schema_version"] = serde_json::json!(2);
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("schema_version 2 refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
}

#[tokio::test]
async fn traversal_path_in_manifest_refuses() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-traversal").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    assert_has_entries(&original);
    // A relative traversal path, then an absolute path, each with COMPLETE
    // refreshed so only the path check can refuse the read.
    for path in ["../x", "/abs"] {
        rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
            value["entries"][0]["path"] = serde_json::json!(path);
        });
        let refused = artifacts
            .read_snapshot(&id)
            .expect_err("a traversal entry path refuses the read");
        assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
    }
}

#[tokio::test]
async fn symlinked_manifest_or_complete_refuses() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-symlink").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    // manifest.json replaced by a symlink to a copy outside the snapshot
    // directory: the reader refuses instead of following it.
    let outside = artifacts_root(&temp).join("outside-manifest");
    fs::write(&outside, &original).unwrap();
    fs::remove_file(&manifest_path).unwrap();
    symlink(&outside, &manifest_path).unwrap();
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("a symlinked manifest refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
    fs::remove_file(&manifest_path).unwrap();
    fs::write(&manifest_path, &original).unwrap();
    // COMPLETE replaced by a symlink to a copy outside the snapshot
    // directory, with the manifest restored: the reader refuses instead of
    // following it.
    let outside_complete = artifacts_root(&temp).join("outside-complete");
    fs::write(&outside_complete, fs::read(&complete_path).unwrap()).unwrap();
    fs::remove_file(&complete_path).unwrap();
    symlink(&outside_complete, &complete_path).unwrap();
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("a symlinked COMPLETE refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
    fs::remove_file(&complete_path).unwrap();
    fs::write(&complete_path, digest(&original).as_str()).unwrap();
    // The snapshot directory itself replaced by a symlink to a copy outside
    // the artifacts root: the reader refuses instead of following it.
    let real = temp.path().join("w04-read-symlink-real");
    fs::rename(&dir, &real).unwrap();
    symlink(&real, &dir).unwrap();
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("a symlinked snapshot directory refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
}

#[tokio::test]
async fn id_mismatch_inside_manifest_refuses() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-id").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let original = fs::read(&manifest_path).unwrap();
    // The manifest claims a different valid snapshot id than the directory
    // it was read from, with COMPLETE refreshed so only the id check can
    // refuse the read.
    rewrite_manifest(&manifest_path, &dir.join("COMPLETE"), &original, |value| {
        value["snapshot_id"] =
            serde_json::json!(SnapshotId::parse("w04-read-other").unwrap().as_str());
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("an id mismatch inside the manifest refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
}

#[tokio::test]
async fn oversize_declared_entry_refuses() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-oversize").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    assert_has_entries(&original);
    // One entry declares 64 MiB + 1 captured bytes — over the per-file
    // bound — with COMPLETE refreshed so only the size bound can refuse
    // the read. The reader treats a per-file bound violation as TooLarge.
    rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
        value["entries"][0]["captured_bytes"] = serde_json::json!(64 * 1024 * 1024 + 1);
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("an over-bound declared size refuses the read");
    assert!(matches!(refused, SnapshotError::TooLarge));
}

#[tokio::test]
async fn duplicate_paths_refuse() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-duplicate").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    assert_has_entries(&original);
    // The same path appears twice, with COMPLETE refreshed so only the
    // duplicate check can refuse the read.
    rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
        let duplicate = value["entries"][0].clone();
        value["entries"].as_array_mut().unwrap().push(duplicate);
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("duplicate entry paths refuse the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
}

#[tokio::test]
async fn nested_extra_field_in_source_refuses() {
    let (temp, artifacts, _captured, id) = publish_head("w04-read-source-extra").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    assert_has_entries(&original);
    // One entry's source object gains an extra field, with COMPLETE
    // refreshed to the new bytes: only the nested source check can refuse
    // the read.
    rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
        value["entries"][0]["source"]["extra"] = serde_json::json!(1);
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("an extra field inside an entry's source refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
}
