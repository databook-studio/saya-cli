use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitService};
use saya_types::ManagedCheckoutId;

use super::{
    state_root,
    support::{TempRoot, current_commit, git_args, git_executable, run_git, setup_repo},
};

#[tokio::test]
async fn aggregate_base_tree_size_is_refused_before_checkout_materialization() {
    let temp = TempRoot::new("tree-byte-limit");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    // Sparse zero-filled blobs exercise Git's reported uncompressed sizes cheaply.
    let file_bytes = 60 * 1024 * 1024;
    for index in 0..9 {
        let path = repo.root.join(format!("large-{index}.bin"));
        fs::File::create(path).unwrap().set_len(file_bytes).unwrap();
    }
    run_git(&git, &repo.home, git_args(&repo.root, &["add", "--", "."]));
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["commit", "--quiet", "-m", "large tree"]),
    );
    let base = current_commit(&git, &repo.home, &repo.root);

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&repo.root, &cancellation)
        .await
        .unwrap();
    let checkout_id = ManagedCheckoutId::parse("checkout-tree-limit").unwrap();
    let path = service.managed_path(&checkout_id).unwrap();
    assert!(matches!(
        service
            .add_worktree(&repository, &base, &checkout_id, &cancellation)
            .await,
        Err(GitError::RepositoryTooLarge)
    ));
    assert!(!Path::new(path.as_str()).exists());
}

#[tokio::test]
async fn aggregate_attribute_scan_size_is_refused_before_checkout_materialization() {
    let temp = TempRoot::new("attribute-byte-limit");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    let contents = vec![b'#'; 900 * 1024];
    for index in 0..5 {
        let directory = repo.root.join(format!("module-{index}"));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(".gitattributes"), &contents).unwrap();
    }
    run_git(&git, &repo.home, git_args(&repo.root, &["add", "--", "."]));
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["commit", "--quiet", "-m", "many attributes"]),
    );
    let base = current_commit(&git, &repo.home, &repo.root);

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&repo.root, &cancellation)
        .await
        .unwrap();
    let checkout_id = ManagedCheckoutId::parse("checkout-attribute-limit").unwrap();
    let path = service.managed_path(&checkout_id).unwrap();
    assert!(matches!(
        service
            .add_worktree(&repository, &base, &checkout_id, &cancellation)
            .await,
        Err(GitError::RepositoryTooLarge)
    ));
    assert!(!Path::new(path.as_str()).exists());
}

#[tokio::test]
async fn attribute_scan_process_count_is_refused_before_checkout_materialization() {
    let temp = TempRoot::new("attribute-file-limit");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    for index in 0..17 {
        let directory = repo.root.join(format!("module-{index}"));
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join(".gitattributes"), b"").unwrap();
    }
    run_git(&git, &repo.home, git_args(&repo.root, &["add", "--", "."]));
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["commit", "--quiet", "-m", "many empty attributes"],
        ),
    );
    let base = current_commit(&git, &repo.home, &repo.root);

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&repo.root, &cancellation)
        .await
        .unwrap();
    let checkout_id = ManagedCheckoutId::parse("checkout-attribute-file-limit").unwrap();
    let path = service.managed_path(&checkout_id).unwrap();
    assert!(matches!(
        service
            .add_worktree(&repository, &base, &checkout_id, &cancellation)
            .await,
        Err(GitError::RepositoryTooLarge)
    ));
    assert!(!Path::new(path.as_str()).exists());
}
