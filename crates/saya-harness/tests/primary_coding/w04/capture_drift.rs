//! Capture drift, quota, and cancellation tests (authored, not executed):
//! a source that changes after planning refuses with the typed identity
//! error and leaves no complete artifact, an over-budget plan is refused
//! before capture can run, and a pre-cancelled token refuses with nothing
//! published — including a valid plan with zero entries. Public API only,
//! real temporary repositories.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::{GitRepository, GitService},
    snapshot::{
        SnapshotArtifacts, SnapshotError, SnapshotManifest, SnapshotOmissionPolicy,
        SnapshotSelection,
    },
};
use saya_harness::workspace::Workspace;
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_executable, repository, state_root};
use super::fixtures::artifacts_root;

/// Plans the fixture's working tree through the public W03 producer with
/// the given snapshot id, letting `prepare` shape the tree before planning.
async fn plan_worktree(
    label: &str,
    prepare: impl FnOnce(&Path),
) -> (TempRoot, GitService, GitRepository, SnapshotManifest) {
    let temp = TempRoot::new(label);
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"planned bytes\n").unwrap();
    prepare(&root);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(label).unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    (temp, service, opened, manifest)
}

#[tokio::test]
async fn source_edit_after_plan_refuses_with_no_complete_artifact() {
    let (temp, service, opened, manifest) = plan_worktree("w04-capture-drift", |_| {}).await;
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    // A selected file's content changes after planning: same path, new
    // bytes with a different size, and the pinned identity includes the
    // size, so it can never match.
    fs::write(opened.root().join("tracked.txt"), b"rewritten after plan\n").unwrap();
    let refused = service
        .capture_snapshot(
            &opened,
            &workspace,
            &manifest,
            &artifacts,
            &CancellationToken::new(),
        )
        .await
        .expect_err("a drifted source refuses the capture");
    assert!(matches!(refused, SnapshotError::SourceIdentityChanged));
    assert!(!artifacts.is_complete(&id).unwrap());
    assert!(!artifacts_root(&temp).join(id.as_str()).exists());
    // The dropped staging is gone: the root holds nothing but its lock.
    assert_eq!(
        super::destination::non_lock_entries(&artifacts_root(&temp)),
        0
    );
}

/// Plan-bound proof only: the planner refuses the over-budget total (the
/// W03 sparse-file proof: 9 × 64 MiB = 576 MiB > 512 MiB) with `TooLarge`
/// before any capture can run, so no artifacts entry can ever appear.
/// This proves W03's plan-level bound, not W04d's capture-level aggregate
/// refusal or staging cleanup: that aggregate check is defence in depth
/// and has no proof at the public boundary.
#[tokio::test]
async fn plan_bound_refuses_oversize_before_capture_can_run() {
    let temp = TempRoot::new("w04-capture-quota");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let directory = root.join("sparse");
    fs::create_dir(&directory).unwrap();
    for index in 0..9 {
        fs::File::create(directory.join(format!("file-{index}")))
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
    }
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let id = SnapshotId::parse("w04-capture-quota").unwrap();
    let planned = service
        .plan_snapshot(
            &opened,
            id.clone(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await;
    assert!(matches!(planned, Err(SnapshotError::TooLarge)));
    assert!(!artifacts.is_complete(&id).unwrap());
    assert!(!artifacts_root(&temp).join(id.as_str()).exists());
    assert_eq!(
        super::destination::non_lock_entries(&artifacts_root(&temp)),
        0
    );
}

#[tokio::test]
async fn cancelled_capture_leaves_no_complete_snapshot() {
    let (temp, service, opened, manifest) = plan_worktree("w04-capture-cancel", |_| {}).await;
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let refused = service
        .capture_snapshot(&opened, &workspace, &manifest, &artifacts, &cancellation)
        .await
        .expect_err("a pre-cancelled capture refuses");
    assert!(matches!(refused, SnapshotError::Cancelled));
    assert!(!artifacts.is_complete(&id).unwrap());
    assert!(!artifacts_root(&temp).join(id.as_str()).exists());
    assert_eq!(
        super::destination::non_lock_entries(&artifacts_root(&temp)),
        0
    );
}

/// The empty-plan cancellation proof: a valid public plan can carry zero
/// entries (the fixture's only tracked file is deleted, which the W03
/// producer records as a tombstone change and never as an entry), and a
/// token cancelled before capture refuses with nothing staged, nothing
/// published, and no artifacts entry.
#[tokio::test]
async fn cancelled_empty_plan_publishes_nothing() {
    let empty_plan = |root: &Path| fs::remove_file(root.join("tracked.txt")).unwrap();
    let (temp, service, opened, manifest) =
        plan_worktree("w04-capture-empty-cancel", empty_plan).await;
    // Premise, asserted so setup drift fails loudly: this public plan is
    // valid and carries zero entries.
    assert!(manifest.entries().is_empty());
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let refused = service
        .capture_snapshot(&opened, &workspace, &manifest, &artifacts, &cancellation)
        .await
        .expect_err("a pre-cancelled empty plan refuses");
    assert!(matches!(refused, SnapshotError::Cancelled));
    assert!(!artifacts.is_complete(&id).unwrap());
    assert!(!artifacts_root(&temp).join(id.as_str()).exists());
    assert_eq!(
        super::destination::non_lock_entries(&artifacts_root(&temp)),
        0
    );
}
