use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{SnapshotError, SnapshotOmissionPolicy, SnapshotOmissionReason, SnapshotSelection},
};
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[tokio::test]
async fn credential_directory_config_is_omitted_by_public_service() {
    let temp = TempRoot::new("snapshot-aws-config");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::create_dir(root.join(".aws")).unwrap();
    fs::write(root.join(".aws/config"), b"fixture profile placeholder\n").unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-aws-config").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    assert!(
        manifest
            .entries()
            .iter()
            .all(|entry| entry.path().as_str() != ".aws/config")
    );
    assert!(manifest.omissions().iter().any(|fact| {
        fact.path().as_str() == ".aws/config"
            && fact.reason() == SnapshotOmissionReason::SensitiveUntracked
    }));
}

#[tokio::test]
async fn staged_and_unstaged_deletions_are_accepted_as_tombstones() {
    let temp = TempRoot::new("snapshot-deletions");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("staged-delete.txt"), b"delete in index\n").unwrap();
    fs::write(root.join("unstaged-delete.txt"), b"delete in worktree\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "staged-delete.txt", "unstaged-delete.txt"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "deletion fixtures"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["rm", "staged-delete.txt"]),
    );
    fs::remove_file(root.join("unstaged-delete.txt")).unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-deletions").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    for path in ["staged-delete.txt", "unstaged-delete.txt"] {
        assert!(
            manifest
                .entries()
                .iter()
                .all(|entry| entry.path().as_str() != path)
        );
        assert!(
            manifest
                .changes()
                .iter()
                .any(|change| change.is_tombstone(path))
        );
    }
}

#[tokio::test]
async fn tracked_sensitive_file_under_private_prefix_refuses_not_omits() {
    let temp = TempRoot::new("snapshot-private-tracked-sensitive");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join(".env.production"), b"fixture placeholder\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", ".env.production"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "tracked env"]),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy =
        SnapshotOmissionPolicy::new(&repository, vec![".env.production".to_owned()]).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-private-tracked-sensitive").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::SensitiveTrackedPath)
    ));
}
