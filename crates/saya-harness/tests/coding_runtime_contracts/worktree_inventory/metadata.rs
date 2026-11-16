use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitBranchState, GitHeadState, GitInventoryError, GitService};

use super::support::{TempRoot, git_args, git_executable, run_git, state_root};

#[tokio::test]
async fn nested_repository_hidden_by_git_status_is_refused_from_metadata_scan() {
    let temp = TempRoot::new("nested");
    let executable = git_executable();
    let root = super::support::repository(temp.path(), &executable);
    let nested = root.join("tracked-directory");
    fs::create_dir(&nested).unwrap();
    fs::write(
        nested.join("tracked.txt"),
        b"tracked before nested repository\n",
    )
    .unwrap();
    run_git(
        &executable,
        temp.path(),
        git_args(&root, &["add", "--", "tracked-directory"]),
    );
    run_git(
        &executable,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "directory"]),
    );
    let template = format!(
        "--template={}",
        temp.path().join("empty-template").display()
    );
    run_git(
        &executable,
        temp.path(),
        git_args(&nested, &["init", "--quiet", &template]),
    );

    let service = GitService::new(&executable, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::NestedRepository)
    ));
    fs::rename(nested.join(".git"), nested.join(".GIT")).unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::NestedRepository)
    ));
}

#[tokio::test]
async fn unborn_head_is_explicit_metadata_absence() {
    let temp = TempRoot::new("unborn");
    let executable = git_executable();
    let root = temp.path().join("unborn repository");
    fs::create_dir(&root).unwrap();
    let template = format!(
        "--template={}",
        temp.path().join("empty-template").display()
    );
    run_git(
        &executable,
        temp.path(),
        git_args(&root, &["init", "--quiet", &template]),
    );

    let service = GitService::new(&executable, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let inventory = service
        .inventory_repository(&opened, &cancellation)
        .await
        .unwrap();
    assert!(matches!(inventory.head(), GitHeadState::Unborn));
    assert!(matches!(inventory.branch(), GitBranchState::Unborn(_)));
    assert!(matches!(
        inventory.worktrees()[0].head(),
        GitHeadState::Unborn
    ));
}
