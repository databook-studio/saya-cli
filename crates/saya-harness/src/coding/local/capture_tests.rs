use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_types::SnapshotId;

use crate::coding::snapshot::SnapshotArtifacts;

use super::identity::LocalRoot;
use super::plan::LocalBaseline;
use super::test_support::Temp;

fn fixture(label: &str) -> (Temp, std::path::PathBuf, std::path::PathBuf) {
    let temp = Temp::new(label);
    let root = temp.mkdir("root");
    let state = temp.mkdir("state");
    (temp, root, state)
}

fn write(root: &Path, name: &str, bytes: &[u8]) {
    let path = root.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(path, bytes).unwrap();
}

/// An edit-time baseline has no entries; the published capture is still an
/// ordinary, complete (empty) snapshot the root verification can read.
#[tokio::test]
async fn empty_baseline_publishes() {
    let (temp, root_path, state) = fixture("capture-empty");
    write(&root_path, "untouched.txt", b"not in the baseline\n");

    let root = LocalRoot::open(&root_path, &state).unwrap();
    let baseline =
        LocalBaseline::edit_time(&root, SnapshotId::parse("job-local-empty").unwrap()).unwrap();
    let artifacts = SnapshotArtifacts::open(&temp.path().join("artifacts")).unwrap();
    let captured = baseline
        .capture(&artifacts, &CancellationToken::new())
        .await
        .unwrap();

    assert_eq!(captured.snapshot_id(), baseline.snapshot_id());
    assert_eq!(captured.repository_id(), baseline.repository_id());
    assert_eq!(captured.plan_manifest_sha256(), baseline.manifest_sha256());
    assert!(captured.entries().is_empty());
    assert!(artifacts.is_complete(baseline.snapshot_id()).unwrap());
}
