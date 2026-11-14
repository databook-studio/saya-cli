//! A real-size repository and a linked worktree are ordinary coding inputs:
//! both must plan, inventory, and open without hitting an opaque bound.
#![cfg(unix)]

use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{SnapshotOmissionPolicy, SnapshotSelection},
};
use saya_types::SnapshotId;

use super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

/// A repository one file above 20,000 files must plan successfully rather
/// than refusing with a generic bound error.
#[tokio::test]
async fn a_repository_above_the_old_snapshot_file_bound_plans_successfully() {
    let temp = TempRoot::new("large-repository");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    for group in 0..21 {
        let directory = root.join("files").join(format!("group-{group:02}"));
        fs::create_dir_all(&directory).unwrap();
        for index in 0..953 {
            fs::File::create(directory.join(format!("file-{index:04}"))).unwrap();
        }
    }
    // 21 * 953 = 20,013 generated files plus `tracked.txt`.
    run_git(&git, temp.path(), git_args(&root, &["add", "--", "."]));
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "large tree"]),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let inventory = service
        .inventory_repository(&repository, &cancellation)
        .await
        .unwrap();
    assert_eq!(inventory.worktrees().len(), 1);

    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-large-repository").unwrap(),
            SnapshotSelection::Head,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    assert_eq!(manifest.entries().len(), 20_014);
}

/// A linked worktree stores its own config in the common git dir and enables
/// `extensions.worktreeconfig`; opening it must resolve that dir and validate
/// the per-worktree config instead of refusing the extension.
#[tokio::test]
async fn a_linked_worktree_with_worktree_config_opens_and_inventories() {
    let temp = TempRoot::new("linked-worktree");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["config", "extensions.worktreeConfig", "true"]),
    );
    let linked = temp.path().join("linked worktree");
    let linked_arg = linked.to_str().unwrap().to_owned();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["worktree", "add", "--detach", &linked_arg]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(
            &linked,
            &["config", "--worktree", "core.sparseCheckout", "false"],
        ),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service
        .open_repository(&linked, &cancellation)
        .await
        .unwrap();
    assert_eq!(
        fs::canonicalize(repository.root()).unwrap(),
        fs::canonicalize(&linked).unwrap()
    );
    let inventory = service
        .inventory_repository(&repository, &cancellation)
        .await
        .unwrap();
    assert_eq!(inventory.worktrees().len(), 2);
}
