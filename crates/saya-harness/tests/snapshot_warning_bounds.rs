//! Snapshot warning bounds: a repository with more sensitive tracked paths
//! than the event bound stores a capped warning list plus one summary count
//! fact, and a plan below the cap carries no summary field.
#![cfg(unix)]

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{
        MAX_SNAPSHOT_WARNINGS, SnapshotManifest, SnapshotOmissionPolicy, SnapshotSelection,
    },
};
use saya_types::SnapshotId;

use support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[path = "snapshot_symlinks/support.rs"]
mod support;

async fn plan(
    temp: &TempRoot,
    root: &Path,
    label: &str,
) -> Result<SnapshotManifest, saya_harness::coding::snapshot::SnapshotError> {
    let git = git_executable();
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(label).unwrap(),
            SnapshotSelection::Head,
            policy,
            &cancellation,
        )
        .await
}

/// One committed credential-shaped file per index, so every path is a tracked
/// sensitive-included warning.
fn sensitive_repository(temp: &TempRoot, git: &Path, count: usize) -> std::path::PathBuf {
    let root = repository(temp.path(), git);
    fs::create_dir(root.join("certs")).unwrap();
    for index in 0..count {
        fs::write(
            root.join(format!("certs/cert-{index:02}.pem")),
            b"fixture placeholder only\n",
        )
        .unwrap();
    }
    run_git(git, temp.path(), git_args(&root, &["add", "certs"]));
    run_git(
        git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "sensitive files"]),
    );
    root
}

#[tokio::test]
async fn warning_facts_are_capped_with_a_summary_count() {
    let temp = TempRoot::new("warning-cap");
    let git = git_executable();
    let root = sensitive_repository(&temp, &git, 70);
    let manifest = plan(&temp, &root, "warning-cap").await.unwrap();
    assert_eq!(manifest.warnings().len(), MAX_SNAPSHOT_WARNINGS);
    assert_eq!(manifest.warnings_truncated(), 6);
    // The stored plan round-trips the summary count fact.
    let restored = SnapshotManifest::from_verified_bytes(
        manifest.canonical_bytes(),
        manifest.manifest_sha256(),
    )
    .unwrap();
    assert_eq!(restored.warnings_truncated(), 6);
}

#[tokio::test]
async fn warning_facts_below_the_cap_have_no_summary_field() {
    let temp = TempRoot::new("warning-under-cap");
    let git = git_executable();
    let root = sensitive_repository(&temp, &git, 3);
    let manifest = plan(&temp, &root, "warning-under-cap").await.unwrap();
    assert_eq!(manifest.warnings().len(), 3);
    assert_eq!(manifest.warnings_truncated(), 0);
    let bytes = String::from_utf8_lossy(manifest.canonical_bytes()).into_owned();
    assert!(
        !bytes.contains("warnings_truncated"),
        "a plan with no truncated warnings keeps its canonical bytes: {bytes}"
    );
}
