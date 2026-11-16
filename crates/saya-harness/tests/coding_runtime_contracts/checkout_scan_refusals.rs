use std::{ffi::CString, fs, os::unix::fs::PermissionsExt, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitConfigRisk, GitError, GitService};
use saya_types::ManagedCheckoutId;

use super::{
    state_root,
    support::{
        TempRoot, current_commit, git_args, git_executable, marker_for, run_git, setup_repo,
        trap_script,
    },
};

#[tokio::test]
async fn configured_filter_is_refused_before_checkout_materialization() {
    let temp = TempRoot::new("filter");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    fs::write(repo.root.join(".gitattributes"), "*.txt filter=hostile\n").unwrap();
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["add", "--", ".gitattributes"]),
    );
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["commit", "--quiet", "-m", "attributes"]),
    );
    let helpers = temp.path().join("filter-helpers");
    fs::create_dir(&helpers).unwrap();
    let smudge = trap_script(&helpers, "smudge");
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["config", "filter.hostile.smudge", smudge.to_str().unwrap()],
        ),
    );
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["config", "filter.hostile.required", "true"]),
    );

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let checkout_id = ManagedCheckoutId::parse("checkout-filter").unwrap();
    let expected_path = service.managed_path(&checkout_id).unwrap();
    assert!(matches!(
        service.open_repository(&repo.root, &cancellation).await,
        Err(GitError::UnsupportedConfiguration(_))
    ));
    assert!(!Path::new(expected_path.as_str()).exists());
    assert!(!marker_for(&smudge).exists());
}

#[tokio::test]
async fn base_tree_filter_attribute_is_refused_before_checkout() {
    let temp = TempRoot::new("tree-filter-attribute");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    fs::write(repo.root.join(".gitattributes"), "*.bin filter=hostile\n").unwrap();
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["add", "--", ".gitattributes"]),
    );
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["commit", "--quiet", "-m", "filter attribute"]),
    );
    let base = current_commit(&git, &repo.home, &repo.root);

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&repo.root, &cancellation)
        .await
        .unwrap();
    let checkout_id = ManagedCheckoutId::parse("checkout-tree-filter").unwrap();
    let path = service.managed_path(&checkout_id).unwrap();
    assert!(matches!(
        service
            .add_worktree(&repository, &base, &checkout_id, &cancellation)
            .await,
        Err(GitError::UnsupportedConfiguration(GitConfigRisk::Filter))
    ));
    assert!(!Path::new(path.as_str()).exists());
}

#[tokio::test]
async fn included_repository_config_is_refused_before_checkout_materialization() {
    let temp = TempRoot::new("include");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    let helpers = temp.path().join("include-helpers");
    fs::create_dir(&helpers).unwrap();
    let hook = trap_script(&helpers, "included-post-checkout");
    let included = temp.path().join("included.gitconfig");
    fs::write(
        &included,
        format!("[core]\n\thooksPath = {}\n", helpers.display()),
    )
    .unwrap();
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &[
                "config",
                "--local",
                "include.path",
                included.to_str().unwrap(),
            ],
        ),
    );

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    assert!(matches!(
        service.open_repository(&repo.root, &cancellation).await,
        Err(GitError::UnsupportedConfiguration(_))
    ));
    let checkout_id = ManagedCheckoutId::parse("checkout-include").unwrap();
    let expected = service.managed_path(&checkout_id).unwrap();
    assert!(!Path::new(expected.as_str()).exists());
    assert!(!marker_for(&hook).exists());
}

#[tokio::test]
async fn lowercase_sparse_index_flag_is_refused_before_checkout_materialization() {
    let temp = TempRoot::new("sparse-lowercase");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["update-index", "--skip-worktree", "--", "tracked.txt"],
        ),
    );
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["update-index", "--assume-unchanged", "--", "tracked.txt"],
        ),
    );

    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&repo.root, &cancellation)
        .await
        .unwrap();
    let checkout_id = ManagedCheckoutId::parse("checkout-sparse-lowercase").unwrap();
    let path = service.managed_path(&checkout_id).unwrap();
    assert!(matches!(
        service
            .add_worktree(&repository, &repo.base, &checkout_id, &cancellation)
            .await,
        Err(GitError::UnsupportedConfiguration(
            saya_harness::coding::git::GitConfigRisk::SparseCheckout
        ))
    ));
    assert!(!Path::new(path.as_str()).exists());
}

#[test]
fn private_git_state_refuses_a_different_owner_when_chown_is_available() {
    let temp = TempRoot::new("state-owner");
    let state = state_root(temp.path());
    let path = CString::new(state.as_os_str().as_encoded_bytes()).unwrap();
    let uid = unsafe { libc::geteuid() };
    let other_uid = if uid == 0 { 1 } else { 0 };
    if unsafe { libc::chown(path.as_ptr(), other_uid, u32::MAX) } != 0 {
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::PermissionDenied,
            "ownership change is unavailable only to an unprivileged test process"
        );
        return;
    }

    let git = git_executable();
    let result = GitService::new(&git, &state);
    let restore = unsafe { libc::chown(path.as_ptr(), uid, u32::MAX) };
    assert_eq!(restore, 0, "test restores its temporary directory owner");
    assert!(matches!(result, Err(GitError::InvalidStateRoot)));
}

#[test]
fn prepopulated_service_hooks_directory_is_refused() {
    let temp = TempRoot::new("prepopulated-hooks");
    let git = git_executable();
    let state = state_root(temp.path());
    let hooks = state.join("empty-hooks");
    fs::create_dir(&hooks).unwrap();
    fs::set_permissions(&hooks, fs::Permissions::from_mode(0o700)).unwrap();
    let hook = trap_script(&hooks, "post-checkout");

    assert!(matches!(
        GitService::new(&git, &state),
        Err(GitError::InvalidStateRoot)
    ));
    assert!(!marker_for(&hook).exists());
}
