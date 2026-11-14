use std::{fs, time::Duration};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitInventoryError, GitService};

use super::support::{
    TempRoot, delayed_git, git_args, git_executable, repository, run_git, state_root,
    wait_group_gone,
};

#[tokio::test]
async fn linked_git_metadata_requests_share_the_inventory_deadline() {
    let temp = TempRoot::new("inventory-deadline");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    for index in 0..10 {
        let linked = temp.path().join(format!("linked-{index}"));
        let mut args = git_args(&root, &["worktree", "add", "--quiet", "--detach"]);
        args.push(linked.as_os_str().to_owned());
        args.push("HEAD".into());
        run_git(&real_git, temp.path(), args);
    }

    let (delayed, marker, pids) = delayed_git(temp.path(), &real_git);
    let service = GitService::new(&delayed, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let started = std::time::Instant::now();
    let result = service.inventory_repository(&opened, &cancellation).await;
    assert!(matches!(
        result,
        Err(GitInventoryError::Git(GitError::TimedOut))
    ));
    assert!(started.elapsed() < Duration::from_secs(35));
    assert!(fs::read(marker).unwrap().len() >= 8);
    let pid_rows = fs::read_to_string(pids).unwrap();
    let process_groups = pid_rows
        .lines()
        .filter_map(|line| line.split_whitespace().next()?.parse::<u32>().ok())
        .collect::<Vec<_>>();
    assert!(process_groups.len() >= 8);
    for process_group in process_groups {
        assert!(wait_group_gone(process_group).await);
    }
}
