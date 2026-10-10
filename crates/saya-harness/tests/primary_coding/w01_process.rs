use std::{sync::Arc, time::Duration};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitService};

use super::{
    state_root,
    support::{TempRoot, assert_processes_gone, fake_git, process_is_alive, wait_for_pids},
};

struct UnrelatedProcess(std::process::Child);

impl Drop for UnrelatedProcess {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn cancellation_reaps_the_owned_git_process_group() {
    let temp = TempRoot::new("cancel-process");
    let script = "#!/bin/sh\n/bin/sleep 30 &\nchild=$!\nprintf '%s %s\\n' \"$$\" \"$child\" > \"$0.pids\"\nwait\n";
    let (git, marker) = fake_git(temp.path(), script);
    let state = state_root(temp.path());
    let service = Arc::new(GitService::new(&git, &state).unwrap());
    let cancellation = CancellationToken::new();
    let child_token = cancellation.clone();
    let task = tokio::spawn(async move { service.git_version(&child_token).await });
    wait_for_marker(&marker).await;
    let pids = wait_for_pids(&marker);
    cancellation.cancel();
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .unwrap()
            .unwrap(),
        Err(GitError::Cancelled)
    ));
    assert_processes_gone(&pids);
}

#[tokio::test]
async fn output_overflow_kills_and_reaps_the_git_process_group() {
    let temp = TempRoot::new("output-limit");
    let script = "#!/bin/sh\n/bin/sleep 30 &\nchild=$!\nprintf '%s %s\\n' \"$$\" \"$child\" > \"$0.pids\"\nexec /bin/dd if=/dev/zero bs=1048576 count=9 2>/dev/null\n";
    let (git, marker) = fake_git(temp.path(), script);
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let result = service.git_version(&cancellation).await;
    let pids = wait_for_pids(&marker);
    assert!(matches!(result, Err(GitError::OutputLimitExceeded)));
    assert_processes_gone(&pids);
}

#[tokio::test]
async fn read_deadline_kills_and_reaps_the_git_process_group() {
    let temp = TempRoot::new("deadline");
    let script = "#!/bin/sh\n/bin/sleep 30 &\nchild=$!\nprintf '%s %s\\n' \"$$\" \"$child\" > \"$0.pids\"\nwait\n";
    let (git, marker) = fake_git(temp.path(), script);
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(async move { service.git_version(&cancellation).await });
    wait_for_marker(&marker).await;
    let pids = wait_for_pids(&marker);
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(12), task)
            .await
            .unwrap()
            .unwrap(),
        Err(GitError::TimedOut)
    ));
    assert_processes_gone(&pids);
}

#[tokio::test]
async fn aborting_git_future_kills_its_process_group_only() {
    let temp = TempRoot::new("abort-process");
    let script = "#!/bin/sh\n/bin/sleep 30 &\nchild=$!\nprintf '%s %s\\n' \"$$\" \"$child\" > \"$0.pids\"\nwait\n";
    let (git, marker) = fake_git(temp.path(), script);
    let state = state_root(temp.path());
    let unrelated = UnrelatedProcess(
        std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap(),
    );
    let unrelated_pid = unrelated.0.id();
    let service = Arc::new(GitService::new(&git, &state).unwrap());
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(async move { service.git_version(&cancellation).await });
    wait_for_marker(&marker).await;
    let pids = wait_for_pids(&marker);

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_processes_gone(&pids);
    assert!(process_is_alive(unrelated_pid));
}

async fn wait_for_marker(marker: &std::path::Path) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Git process starts and records its children");
}
