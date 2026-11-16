use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitService};
use saya_types::{GitObjectId, ManagedCheckoutId};

use super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

fn head_commit(git: &Path, fixture_root: &Path, repository_root: &Path) -> GitObjectId {
    let output = run_git(
        git,
        fixture_root,
        git_args(repository_root, &["rev-parse", "--verify", "HEAD^{commit}"]),
    );
    GitObjectId::parse(String::from_utf8(output).unwrap().trim()).unwrap()
}

#[tokio::test]
async fn no_checkout_leaves_dot_git_only() {
    let temp = TempRoot::new("w05-no-checkout");
    let git = git_executable();
    let repo_root = repository(temp.path(), &git);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repo = service
        .open_repository(&repo_root, &cancellation)
        .await
        .unwrap();
    let base = head_commit(&git, temp.path(), &repo_root);
    let checkout_id = ManagedCheckoutId::parse("checkout-w05-a").unwrap();
    let worktree = service
        .add_worktree_no_checkout(&repo, &base, &checkout_id, &cancellation)
        .await
        .unwrap();
    let path = Path::new(worktree.path().as_str());
    let mut entries: Vec<String> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    entries.sort();
    assert_eq!(entries, vec![".git"]);
    let head = run_git(
        &git,
        temp.path(),
        git_args(path, &["rev-parse", "--verify", "HEAD^{commit}"]),
    );
    assert_eq!(String::from_utf8(head).unwrap().trim(), base.as_str());
    let branch = run_git(
        &git,
        temp.path(),
        git_args(path, &["rev-parse", "--abbrev-ref", "HEAD"]),
    );
    assert_eq!(String::from_utf8(branch).unwrap().trim(), "HEAD");
}

#[tokio::test]
async fn existing_path_refuses_no_checkout() {
    let temp = TempRoot::new("w05-existing-path");
    let git = git_executable();
    let repo_root = repository(temp.path(), &git);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repo = service
        .open_repository(&repo_root, &cancellation)
        .await
        .unwrap();
    let base = head_commit(&git, temp.path(), &repo_root);
    let checkout_id = ManagedCheckoutId::parse("checkout-w05-b").unwrap();
    let managed_path = service.managed_path(&checkout_id).unwrap();
    fs::create_dir_all(Path::new(managed_path.as_str())).unwrap();
    let result = service
        .add_worktree_no_checkout(&repo, &base, &checkout_id, &cancellation)
        .await;
    assert!(matches!(result, Err(GitError::ManagedPathAlreadyExists)));
}

#[tokio::test]
async fn adopt_identical_worktree_succeeds() {
    let temp = TempRoot::new("w05-adopt-identical");
    let git = git_executable();
    let repo_root = repository(temp.path(), &git);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repo = service
        .open_repository(&repo_root, &cancellation)
        .await
        .unwrap();
    let base = head_commit(&git, temp.path(), &repo_root);
    let checkout_id = ManagedCheckoutId::parse("checkout-w05-c").unwrap();
    let created = service
        .add_worktree_no_checkout(&repo, &base, &checkout_id, &cancellation)
        .await
        .unwrap();
    let adopted = service
        .adopt_worktree(&repo, &base, &checkout_id, &cancellation)
        .await
        .unwrap();
    assert_eq!(adopted.path().as_str(), created.path().as_str());
    assert_eq!(adopted.base_commit().as_str(), base.as_str());
    assert_eq!(adopted.checkout_id().as_str(), checkout_id.as_str());
}

#[tokio::test]
async fn adopt_mismatched_refuses() {
    let temp = TempRoot::new("w05-adopt-mismatch");
    let git = git_executable();
    let repo_root = repository(temp.path(), &git);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repo = service
        .open_repository(&repo_root, &cancellation)
        .await
        .unwrap();
    let base = head_commit(&git, temp.path(), &repo_root);

    let plain_id = ManagedCheckoutId::parse("checkout-w05-plain").unwrap();
    let plain_path = service.managed_path(&plain_id).unwrap();
    fs::create_dir_all(Path::new(plain_path.as_str())).unwrap();
    let plain = service
        .adopt_worktree(&repo, &base, &plain_id, &cancellation)
        .await;
    assert!(matches!(
        plain,
        Err(GitError::MutationOutcomeUnknown { .. })
    ));

    fs::write(repo_root.join("tracked.txt"), b"second\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&repo_root, &["add", "--", "tracked.txt"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&repo_root, &["commit", "--quiet", "-m", "second"]),
    );
    let moved = head_commit(&git, temp.path(), &repo_root);
    let head_id = ManagedCheckoutId::parse("checkout-w05-head").unwrap();
    let head_path = service.managed_path(&head_id).unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(
            &repo_root,
            &[
                "worktree",
                "add",
                "--detach",
                "--",
                head_path.as_str(),
                moved.as_str(),
            ],
        ),
    );
    let result = service
        .adopt_worktree(&repo, &base, &head_id, &cancellation)
        .await;
    match result {
        Ok(_) => panic!("adoption must refuse a worktree whose HEAD differs"),
        Err(GitError::MutationOutcomeUnknown { source, .. }) => {
            assert!(matches!(*source, GitError::RepositoryChanged));
        }
        Err(other) => panic!("unexpected adoption error: {other:?}"),
    }
}
