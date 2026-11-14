use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitBranchState, GitHeadState, GitInventoryError, GitService};

use super::support::{
    TempRoot, git_args, git_executable, repository, response_git, run_git, state_root,
};

#[tokio::test]
async fn inventory_reports_staged_unstaged_untracked_and_ignored_without_mutation() {
    let temp = TempRoot::new("status");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"staged\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "--", "tracked.txt"]),
    );
    fs::write(root.join("tracked.txt"), b"unstaged after staging\n").unwrap();
    fs::write(root.join(".gitignore"), b"ignored.txt\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "--", ".gitignore"]),
    );
    fs::write(root.join("ignored.txt"), b"ignored\n").unwrap();
    fs::write(root.join("untracked file.txt"), b"untracked\n").unwrap();
    let status_args = [
        "status",
        "--porcelain=v2",
        "-z",
        "--branch",
        "--untracked-files=all",
        "--ignored=matching",
    ];
    let before_status = run_git(&git, temp.path(), git_args(&root, &status_args));
    let before_index = fs::read(root.join(".git/index")).unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let inventory = service
        .inventory_repository(&opened, &cancellation)
        .await
        .unwrap();
    let status = |path: &str| {
        inventory
            .paths()
            .iter()
            .find(|entry| entry.path() == std::path::Path::new(path))
            .unwrap()
    };
    assert!(status("tracked.txt").staged() && status("tracked.txt").unstaged());
    assert!(status("untracked file.txt").untracked());
    assert!(status("ignored.txt").ignored());
    assert!(matches!(inventory.head(), GitHeadState::Commit(_)));
    assert!(matches!(inventory.branch(), GitBranchState::Attached(_)));
    assert_eq!(
        before_status,
        run_git(&git, temp.path(), git_args(&root, &status_args))
    );
    assert_eq!(before_index, fs::read(root.join(".git/index")).unwrap());
}

#[tokio::test]
async fn malformed_status_protocol_is_refused_at_the_public_service_boundary() {
    let temp = TempRoot::new("malformed-status");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    let response = b"# branch.oid malformed-object\0# branch.head main\0";
    let wrapper = response_git(temp.path(), &real_git, "status", response);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::MalformedOutput)
    ));
}
