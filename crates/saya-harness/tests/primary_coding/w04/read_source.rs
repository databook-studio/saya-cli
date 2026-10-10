//! Working-tree source read-back strictness (authored, not executed): a
//! published working-tree entry whose source object carries an extra
//! field refuses with the typed malformed error, even though COMPLETE is
//! refreshed to the new bytes. Public API only, real temporary
//! repositories.

use std::fs;

use saya_harness::coding::snapshot::SnapshotError;

use super::fixtures::artifacts_root;
use super::read::publish_current;
use super::read_corrupt::{assert_has_entries, rewrite_manifest};

#[tokio::test]
async fn working_tree_extra_field_in_source_refuses() {
    let (temp, artifacts, _captured, id) = publish_current("w04-read-wt-extra").await;
    let dir = artifacts_root(&temp).join(id.as_str());
    let manifest_path = dir.join("manifest.json");
    let complete_path = dir.join("COMPLETE");
    let original = fs::read(&manifest_path).unwrap();
    assert_has_entries(&original);
    // One entry's working-tree source gains an extra field, with COMPLETE
    // refreshed to the new bytes: only the stored source parse can refuse
    // the read.
    rewrite_manifest(&manifest_path, &complete_path, &original, |value| {
        value["entries"][0]["source"] = serde_json::json!({"kind": "working_tree", "extra": 1});
    });
    let refused = artifacts
        .read_snapshot(&id)
        .expect_err("an extra field in a working-tree source refuses the read");
    assert!(matches!(refused, SnapshotError::CapturedManifestMalformed));
}
