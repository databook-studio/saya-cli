use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitBranchState, GitHeadState, GitService};

use super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[tokio::test]
async fn linked_worktree_identity_and_newline_paths_are_inventory_metadata() {
    let temp = TempRoot::new("linked");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let linked = temp.path().join("linked\nworktree");
    let ordinary_linked = temp.path().join("linked-worktree");
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["update-ref", "refs/heads/HEAD", "HEAD"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["symbolic-ref", "HEAD", "refs/heads/HEAD"]),
    );
    for path in [&linked, &ordinary_linked] {
        let mut add = git_args(&root, &["worktree", "add", "--quiet", "--detach"]);
        add.push(path.as_os_str().to_owned());
        add.push("HEAD".into());
        run_git(&git, temp.path(), add);
    }
    fs::write(root.join("space and\nnewline.txt"), b"metadata only\n").unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let inventory = service
        .inventory_repository(&opened, &cancellation)
        .await
        .unwrap();
    let linked_record = inventory
        .worktrees()
        .iter()
        .find(|item| item.path() == linked.as_path())
        .unwrap();
    assert_eq!(linked_record.head(), inventory.head());
    assert!(
        matches!(inventory.branch(), GitBranchState::Attached(name) if name.as_str() == "HEAD")
    );
    assert!(matches!(linked_record.branch(), GitBranchState::Detached));
    assert!(matches!(linked_record.head(), GitHeadState::Commit(_)));
    assert_eq!(inventory.worktrees().len(), 3);
    assert_eq!(inventory.paths().len(), 1);
    assert_eq!(
        inventory.paths()[0].path(),
        std::path::Path::new("space and\nnewline.txt")
    );

    let linked_open = service
        .open_repository(&ordinary_linked, &cancellation)
        .await
        .unwrap();
    let linked_inventory = service
        .inventory_repository(&linked_open, &cancellation)
        .await
        .unwrap();
    assert_eq!(
        inventory.identity().repository_id(),
        linked_inventory.identity().repository_id()
    );
    assert_eq!(
        inventory.identity().common_dir_sha256(),
        linked_inventory.identity().common_dir_sha256()
    );
}
