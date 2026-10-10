//! Published-snapshot read-back tests (authored, not executed): the public
//! W04 reader returns exactly what `GitService::capture_snapshot` published
//! for the snapshot id — entries, digests, and byte counts — including a
//! Head capture sourced from committed Git blobs, and refuses an id that
//! was never published with the typed not-found error. Public API only,
//! real temporary repositories.

use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{
        CapturedSnapshot, SnapshotArtifacts, SnapshotContentSource, SnapshotError,
        SnapshotOmissionPolicy, SnapshotSelection,
    },
};
use saya_harness::workspace::Workspace;
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};
use super::fixtures::{artifacts_root, digest};

/// Builds one real Git repository with `tracked.txt`, committing it first
/// for a Head selection, plans it through the public W03 producer, and
/// captures it through the public W04d producer into the fixture artifacts
/// root, so every test starts from exactly what capture published.
async fn publish_selection(
    label: &str,
    selection: SnapshotSelection,
) -> (TempRoot, SnapshotArtifacts, CapturedSnapshot, SnapshotId) {
    let temp = TempRoot::new(label);
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"read fixture bytes\n").unwrap();
    if selection == SnapshotSelection::Head {
        run_git(
            &git,
            temp.path(),
            git_args(&root, &["add", "--", "tracked.txt"]),
        );
        run_git(
            &git,
            temp.path(),
            git_args(&root, &["commit", "--quiet", "-m", "tracked"]),
        );
    }
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(label).unwrap(),
            selection,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = service
        .capture_snapshot(&opened, &workspace, &manifest, &artifacts, &cancellation)
        .await
        .unwrap();
    (temp, artifacts, captured, id)
}

/// Publishes the current working content: the default fixture for these
/// read-back tests.
pub(super) async fn publish_current(
    label: &str,
) -> (TempRoot, SnapshotArtifacts, CapturedSnapshot, SnapshotId) {
    publish_selection(label, SnapshotSelection::CurrentWorkingContent).await
}

/// Publishes a Head capture: entries are sourced from committed Git blobs.
pub(super) async fn publish_head(
    label: &str,
) -> (TempRoot, SnapshotArtifacts, CapturedSnapshot, SnapshotId) {
    publish_selection(label, SnapshotSelection::Head).await
}

#[tokio::test]
async fn published_snapshot_reads_back_exactly() {
    let (temp, artifacts, captured, id) = publish_current("w04-read-exact").await;
    // Premise, asserted so setup drift fails loudly: the published
    // directory is complete before anything reads it.
    assert!(artifacts.is_complete(&id).unwrap());
    let read_back = artifacts.read_snapshot(&id).unwrap();
    // Premise for the entry loops below: capture published at least one
    // entry.
    assert!(!captured.entries().is_empty());
    // Full equality: the read-back is exactly the value capture returned,
    // beyond the per-field checks that follow.
    assert_eq!(read_back, captured);
    // The captured manifest digest matches the capture result and,
    // independently, the exact stored manifest.json bytes.
    assert_eq!(
        read_back.captured_manifest_sha256().as_str(),
        captured.captured_manifest_sha256().as_str()
    );
    let manifest = fs::read(
        artifacts_root(&temp)
            .join(id.as_str())
            .join("manifest.json"),
    )
    .unwrap();
    assert_eq!(
        digest(&manifest).as_str(),
        read_back.captured_manifest_sha256().as_str()
    );
    // Every entry reads back with its captured path, mode, content digest,
    // and byte count, in capture order.
    assert_eq!(read_back.entries().len(), captured.entries().len());
    for (captured_entry, read_entry) in captured.entries().iter().zip(read_back.entries()) {
        assert_eq!(captured_entry.path().as_str(), read_entry.path().as_str());
        assert_eq!(captured_entry.mode(), read_entry.mode());
        assert_eq!(
            captured_entry.content_sha256().as_str(),
            read_entry.content_sha256().as_str()
        );
        assert_eq!(captured_entry.captured_bytes(), read_entry.captured_bytes());
    }
    // Each object on disk is the content-addressed payload the entry names.
    let objects = artifacts_root(&temp).join(id.as_str()).join("objects");
    for read_entry in read_back.entries() {
        let stored = fs::read(objects.join(read_entry.content_sha256().as_str())).unwrap();
        assert_eq!(stored.len() as u64, read_entry.captured_bytes());
        assert_eq!(
            digest(&stored).as_str(),
            read_entry.content_sha256().as_str()
        );
    }
}

/// A Head capture publishes entries sourced from committed Git blobs, and
/// the published snapshot reads back exactly the capture result.
#[tokio::test]
async fn head_capture_reads_back_with_git_blob_sources() {
    let (_temp, artifacts, captured, id) = publish_head("w04-read-head").await;
    assert!(artifacts.is_complete(&id).unwrap());
    let read_back = artifacts.read_snapshot(&id).unwrap();
    // Full equality with what Head capture returned.
    assert_eq!(read_back, captured);
    // Head capture took the entries from committed Git blobs, not from the
    // working tree.
    assert!(
        captured
            .entries()
            .iter()
            .any(|entry| matches!(entry.source(), SnapshotContentSource::GitBlob { .. }))
    );
}

#[tokio::test]
async fn unknown_snapshot_refuses() {
    let (_temp, artifacts, _captured, id) = publish_current("w04-read-unknown").await;
    let ghost = SnapshotId::parse("w04-read-ghost").unwrap();
    let refused = artifacts
        .read_snapshot(&ghost)
        .expect_err("an unpublished snapshot id refuses the read");
    assert!(matches!(refused, SnapshotError::SnapshotNotFound));
    // The refusal tracks the missing id alone: the published snapshot still
    // reads back.
    assert!(artifacts.read_snapshot(&id).is_ok());
}
