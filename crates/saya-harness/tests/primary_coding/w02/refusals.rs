use std::{fs, io::Write as _};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitInventoryError, GitService};

use super::support::{
    TempRoot, git_args, git_executable, repository, response_git, run_git, state_root,
};

#[tokio::test]
async fn foreign_worktree_record_is_refused_by_git_directory_identity() {
    let temp = TempRoot::new("foreign-worktree");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    let foreign_parent = temp.path().join("foreign");
    fs::create_dir(&foreign_parent).unwrap();
    let foreign = repository(&foreign_parent, &real_git);
    let head = git_text(
        &real_git,
        temp.path(),
        &root,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    );
    let foreign_head = git_text(
        &real_git,
        temp.path(),
        &foreign,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    );
    let branch = git_text(&real_git, temp.path(), &root, &["branch", "--show-current"]);
    let foreign_branch = git_text(
        &real_git,
        temp.path(),
        &foreign,
        &["branch", "--show-current"],
    );
    let root = root.canonicalize().unwrap();
    let foreign = foreign.canonicalize().unwrap();
    let response = format!(
        "worktree {}\0HEAD {head}\0branch refs/heads/{branch}\0\0worktree {}\0HEAD {foreign_head}\0branch refs/heads/{foreign_branch}\0\0",
        root.display(),
        foreign.display()
    );
    let wrapper = response_git(temp.path(), &real_git, "worktree", response.as_bytes());
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::UnsupportedWorktree)
    ));
}

#[tokio::test]
async fn bare_repository_is_refused_before_inventory() {
    let temp = TempRoot::new("bare");
    let real_git = git_executable();
    let bare = temp.path().join("bare.git");
    fs::create_dir(&bare).unwrap();
    let template = format!(
        "--template={}",
        temp.path().join("empty-template").display()
    );
    run_git(
        &real_git,
        temp.path(),
        git_args(&bare, &["init", "--bare", "--quiet", &template]),
    );
    let service = GitService::new(&real_git, state_root(temp.path())).unwrap();
    assert!(matches!(
        service
            .open_repository(&bare, &CancellationToken::new())
            .await,
        Err(GitError::BareRepository)
    ));
}

#[tokio::test]
async fn inventory_bounds_paths_aggregated_across_status_and_index() {
    let temp = TempRoot::new("path-budget");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    let head = git_text(
        &real_git,
        temp.path(),
        &root,
        &["rev-parse", "--verify", "HEAD^{commit}"],
    );
    let branch = git_text(&real_git, temp.path(), &root, &["branch", "--show-current"]);
    let mut response = format!("# branch.oid {head}\0# branch.head {branch}\0").into_bytes();
    for index in 0..19_999 {
        write!(response, "? untracked-{index:05}\0").unwrap();
    }
    let wrapper = response_git(temp.path(), &real_git, "status", &response);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::TooLarge)
    ));
}

fn git_text(
    executable: &std::path::Path,
    fixture: &std::path::Path,
    root: &std::path::Path,
    args: &[&str],
) -> String {
    String::from_utf8(run_git(executable, fixture, git_args(root, args)))
        .unwrap()
        .trim()
        .to_owned()
}
