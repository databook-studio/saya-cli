use std::fs::{self, File};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{SnapshotError, SnapshotOmissionPolicy, SnapshotSelection},
};
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_args, git_executable, repository, state_root};

#[tokio::test]
async fn single_file_over_64_mib_refuses_before_content_reads() {
    let temp = TempRoot::new("snapshot-file-limit");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let file = File::create(root.join("large.bin")).unwrap();
    file.set_len(64 * 1024 * 1024 + 1).unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-file-limit").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::TooLarge)
    ));
}

#[tokio::test]
async fn aggregate_content_over_512_mib_refuses_with_sparse_files() {
    let temp = TempRoot::new("snapshot-total-limit");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let directory = root.join("sparse");
    fs::create_dir(&directory).unwrap();
    for index in 0..9 {
        File::create(directory.join(format!("file-{index}")))
            .unwrap()
            .set_len(64 * 1024 * 1024)
            .unwrap();
    }

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-total-limit").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::TooLarge)
    ));
}

#[tokio::test]
async fn count_limit_refuses_the_first_path_over_20000() {
    let temp = TempRoot::new("snapshot-count-limit");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    super::super::support::run_git(&git, temp.path(), git_args(&root, &["rm", "tracked.txt"]));
    super::super::support::run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "empty tree"]),
    );
    for index in 0..20_001 {
        File::create(root.join(format!("untracked-{index:05}"))).unwrap();
    }

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-count-limit").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::TooLarge)
    ));
}

#[tokio::test]
async fn serialized_manifest_metadata_is_capped_at_8_mib() {
    let temp = TempRoot::new("snapshot-manifest-limit");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    super::super::support::run_git(&git, temp.path(), git_args(&root, &["rm", "tracked.txt"]));
    super::super::support::run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "empty tree"]),
    );
    let outer = root.join("a".repeat(240));
    let inner = outer.join("b".repeat(225));
    fs::create_dir_all(&inner).unwrap();
    for index in 0..7_500 {
        File::create(inner.join(format!("file-{index:05}"))).unwrap();
    }

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-manifest-limit").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::TooLarge)
    ));
}
