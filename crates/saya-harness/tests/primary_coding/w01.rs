#![cfg(unix)]

#[path = "w01_process.rs"]
mod process;
#[path = "w01_support.rs"]
mod support;

use std::os::unix::fs::PermissionsExt;
use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitService};
use saya_types::ManagedCheckoutId;
use support::{TempRoot, git_args, git_executable, marker_for, run_git, setup_repo, trap_script};

fn state_root(root: &Path) -> std::path::PathBuf {
    let state = root.join("app-state");
    fs::create_dir(&state).expect("private state root is new");
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))
        .expect("private state root has mode 0700");
    state
}

#[tokio::test]
async fn detached_worktree_neutralizes_helpers_and_preserves_original_state() {
    let temp = TempRoot::new("lifecycle");
    let git = git_executable();
    let repo = setup_repo(temp.path(), &git);
    fs::write(repo.root.join("tracked.txt"), b"staged content\n").unwrap();
    run_git(
        &git,
        &repo.home,
        git_args(&repo.root, &["add", "--", "tracked.txt"]),
    );
    fs::write(repo.root.join("tracked.txt"), b"unstaged content\n").unwrap();
    fs::write(repo.root.join("untracked.txt"), b"untracked content\n").unwrap();
    let status = run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
        ),
    );
    let index = fs::read(repo.root.join(".git/index")).unwrap();

    let helpers = temp.path().join("helpers");
    fs::create_dir(&helpers).unwrap();
    let hook = trap_script(&helpers, "post-checkout");
    let fsmonitor = trap_script(&helpers, "fsmonitor");
    let external_diff = trap_script(&helpers, "external-diff");
    let credential = trap_script(&helpers, "credential");
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["config", "core.hooksPath", helpers.to_str().unwrap()],
        ),
    );
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["config", "core.fsmonitor", fsmonitor.to_str().unwrap()],
        ),
    );
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["config", "diff.external", external_diff.to_str().unwrap()],
        ),
    );
    let credential_value = format!("!{}", credential.display());
    run_git(
        &git,
        &repo.home,
        git_args(
            &repo.root,
            &["config", "credential.helper", &credential_value],
        ),
    );

    let state = state_root(temp.path());
    let global_hook = trap_script(&helpers, "global-post-checkout");
    fs::write(
        state.join(".gitconfig"),
        format!(
            "[core]\n\thooksPath = {}\n\tfsmonitor = {}\n[diff]\n\texternal = {}\n",
            helpers.display(),
            global_hook.display(),
            external_diff.display()
        ),
    )
    .unwrap();
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&repo.root, &cancellation)
        .await
        .unwrap();
    let checkout_id = ManagedCheckoutId::parse("checkout-w01-a").unwrap();
    let worktree = service
        .add_worktree(&repository, &repo.base, &checkout_id, &cancellation)
        .await
        .unwrap();
    let worktree_path = Path::new(worktree.path().as_str());
    assert!(worktree_path.starts_with(state.join("worktrees")));
    let head = run_git(
        &git,
        &repo.home,
        git_args(worktree_path, &["rev-parse", "--verify", "HEAD^{commit}"]),
    );
    assert_eq!(String::from_utf8(head).unwrap().trim(), repo.base.as_str());
    let symbolic = support::try_git(
        &git,
        &repo.home,
        git_args(worktree_path, &["symbolic-ref", "-q", "HEAD"]),
    );
    assert!(!symbolic.status.success(), "the new worktree is detached");
    for script in [&hook, &fsmonitor, &external_diff, &credential, &global_hook] {
        assert!(
            !marker_for(script).exists(),
            "Git did not execute configured helper"
        );
    }

    fs::write(
        worktree_path.join("dirty.txt"),
        b"keep until owner cleans\n",
    )
    .unwrap();
    assert!(matches!(
        service.remove_worktree(&worktree, &cancellation).await,
        Err(GitError::WorktreeNotClean)
    ));
    assert!(worktree_path.exists());
    fs::remove_file(worktree_path.join("dirty.txt")).unwrap();
    service
        .remove_worktree(&worktree, &cancellation)
        .await
        .unwrap();
    assert!(!worktree_path.exists());
    assert_eq!(
        run_git(
            &git,
            &repo.home,
            git_args(
                &repo.root,
                &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            ),
        ),
        status
    );
    assert_eq!(fs::read(repo.root.join(".git/index")).unwrap(), index);
}

#[path = "w01_limits.rs"]
mod limits;
#[path = "w01_refusal.rs"]
mod refusal;
