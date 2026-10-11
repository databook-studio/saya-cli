use std::{fs, sync::Arc, time::Duration};

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitProcessEffect, GitProcessOutcome, GitService};

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
    let observer = Arc::clone(&service);
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
    assert_eq!(
        observer
            .process_observations()
            .receipts()
            .last()
            .unwrap()
            .outcome(),
        GitProcessOutcome::Cancelled
    );
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
    assert_eq!(
        service
            .process_observations()
            .receipts()
            .last()
            .unwrap()
            .outcome(),
        GitProcessOutcome::OutputLimited
    );
}

#[tokio::test]
async fn read_deadline_kills_and_reaps_the_git_process_group() {
    let temp = TempRoot::new("deadline");
    let script = "#!/bin/sh\n/bin/sleep 30 &\nchild=$!\nprintf '%s %s\\n' \"$$\" \"$child\" > \"$0.pids\"\nwait\n";
    let (git, marker) = fake_git(temp.path(), script);
    let state = state_root(temp.path());
    let service = Arc::new(GitService::new(&git, &state).unwrap());
    let observer = Arc::clone(&service);
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
    assert_eq!(
        observer
            .process_observations()
            .receipts()
            .last()
            .unwrap()
            .outcome(),
        GitProcessOutcome::TimedOut
    );
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
    let observer = Arc::clone(&service);
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(async move { service.git_version(&cancellation).await });
    wait_for_marker(&marker).await;
    let pids = wait_for_pids(&marker);

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_processes_gone(&pids);
    assert!(process_is_alive(unrelated_pid));
    assert_eq!(
        observer
            .process_observations()
            .receipts()
            .last()
            .unwrap()
            .outcome(),
        GitProcessOutcome::Unknown
    );
}

#[tokio::test]
async fn observer_receipts_are_bounded_and_truthful_about_local_git_lifecycle() {
    let temp = TempRoot::new("observer-receipts");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let token = CancellationToken::new();
    assert_eq!(
        service.git_version(&token).await.unwrap(),
        "git version 2.50.1"
    );
    let snapshot = service.process_observations();
    assert!(snapshot.complete());
    assert!(!snapshot.service_identity().is_empty());
    let receipt = snapshot.receipts().last().unwrap();
    assert_eq!(receipt.version(), 1);
    assert_eq!(receipt.service_identity(), snapshot.service_identity());
    assert_eq!(receipt.request_kind(), "version");
    assert_eq!(receipt.outcome(), GitProcessOutcome::Exited);
    assert_eq!(receipt.effect(), GitProcessEffect::None);
    assert_eq!(receipt.signal(), None);
    assert!(receipt.pid().is_some());
    assert_eq!(receipt.exit_code(), Some(0));
    assert!(!receipt.stdout_truncated());
    assert!(!receipt.stderr_truncated());
    assert!(receipt.stdout_bytes() > 0);
}

#[tokio::test]
async fn observer_records_non_success_without_claiming_mutation_effects() {
    let temp = TempRoot::new("observer-failures");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nexit 9\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let token = CancellationToken::new();
    assert!(matches!(
        service.git_version(&token).await,
        Err(GitError::UnsupportedGitVersion)
    ));
    let receipt = service
        .process_observations()
        .receipts()
        .last()
        .unwrap()
        .clone();
    assert_eq!(receipt.outcome(), GitProcessOutcome::Exited);
    assert_eq!(receipt.effect(), GitProcessEffect::None);
    assert_eq!(receipt.exit_code(), Some(9));
}

#[tokio::test]
async fn observer_records_spawn_failure_without_a_pid() {
    let temp = TempRoot::new("observer-spawn");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nexit 0\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    std::fs::remove_file(git).unwrap();
    assert!(matches!(
        service.git_version(&CancellationToken::new()).await,
        Err(GitError::Spawn(_))
    ));
    let receipt = service.process_observations().receipts().last().unwrap();
    assert_eq!(receipt.outcome(), GitProcessOutcome::SpawnFailed);
    assert_eq!(receipt.pid(), None);
    assert_eq!(receipt.killed_orphans(), None);
}

#[tokio::test]
async fn observer_retains_signal_and_omits_over_its_journal_byte_bound() {
    let temp = TempRoot::new("observer-signal-bound");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nkill -TERM $$\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    assert!(
        service
            .git_version(&CancellationToken::new())
            .await
            .is_err()
    );
    let receipt = service
        .process_observations()
        .receipts()
        .last()
        .unwrap()
        .clone();
    assert_eq!(receipt.outcome(), GitProcessOutcome::Exited);
    assert_eq!(receipt.signal(), Some(15));
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let service = GitService::new(&git, &state).unwrap();
    for _ in 0..257 {
        service
            .git_version(&CancellationToken::new())
            .await
            .unwrap();
    }
    let snapshot = service.process_observations();
    assert!(!snapshot.complete());
    assert!(snapshot.omitted() > 0);
    assert!(snapshot.omitted_bytes() > 0);
}

#[tokio::test]
async fn process_journal_reopens_receipts_without_raw_data() {
    let temp = TempRoot::new("journal-reopen");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let state = state_root(temp.path());
    let first = GitService::new(&git, &state).unwrap();
    first.git_version(&CancellationToken::new()).await.unwrap();
    let first_identity = first.process_observations().service_identity().to_owned();
    let bytes = fs::read(state.join("git-processes.jsonl")).unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("arguments"));
    assert!(!String::from_utf8_lossy(&bytes).contains("git version 2.50.1"));
    drop(first);

    let second = GitService::new(&git, &state).unwrap();
    let reopened = second.process_observations();
    assert!(reopened.complete());
    assert!(reopened.receipts().iter().any(|receipt| {
        receipt.service_identity() == first_identity && receipt.sequence() == 1
    }));
    second.git_version(&CancellationToken::new()).await.unwrap();
    assert!(
        second
            .process_observations()
            .receipts()
            .iter()
            .any(|receipt| receipt.service_identity() != first_identity)
    );
}

#[tokio::test]
async fn malformed_process_journal_stays_incomplete_without_reset() {
    let temp = TempRoot::new("journal-malformed");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    service
        .git_version(&CancellationToken::new())
        .await
        .unwrap();
    let journal = state.join("git-processes.jsonl");
    let mut file = fs::OpenOptions::new().append(true).open(&journal).unwrap();
    use std::io::Write as _;
    file.write_all(b"{\"unknown\":true}\n").unwrap();
    file.sync_all().unwrap();
    drop(service);

    let reopened = GitService::new(&git, &state).unwrap();
    let snapshot = reopened.process_observations();
    assert!(!snapshot.complete());
    assert!(!snapshot.receipts().is_empty());
    let before = fs::read(&journal).unwrap();
    reopened
        .git_version(&CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fs::read(&journal).unwrap(), before);
    assert!(!reopened.process_observations().complete());
}

#[tokio::test]
async fn malformed_signal_marks_shared_journal_incomplete() {
    let temp = TempRoot::new("journal-signal");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    service
        .git_version(&CancellationToken::new())
        .await
        .unwrap();
    drop(service);
    let journal = state.join("git-processes.jsonl");
    let line = fs::read_to_string(&journal).unwrap();
    fs::write(&journal, line.replace("\"signal\":null", "\"signal\":0")).unwrap();
    let reopened = GitService::new(&git, &state).unwrap();
    assert!(!reopened.process_observations().complete());
    assert!(reopened.process_observations().receipts().is_empty());
}

#[tokio::test]
async fn two_services_share_the_journal_bound() {
    let temp = TempRoot::new("journal-shared");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let state = state_root(temp.path());
    let first = GitService::new(&git, &state).unwrap();
    let second = GitService::new(&git, &state).unwrap();
    for index in 0..255 {
        let service = if index % 2 == 0 { &first } else { &second };
        service
            .git_version(&CancellationToken::new())
            .await
            .unwrap();
    }
    let first_snapshot = first.process_observations();
    let second_snapshot = second.process_observations();
    assert!(!first_snapshot.complete() || !second_snapshot.complete());
    drop(first);
    drop(second);
    let reopened = GitService::new(&git, &state).unwrap();
    let snapshot = reopened.process_observations();
    assert!(!snapshot.complete());
    assert!(snapshot.receipts().len() <= 256);
}

#[tokio::test]
async fn process_journal_omission_survives_reopen() {
    let temp = TempRoot::new("journal-bound");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.50.1\\n'\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    for _ in 0..257 {
        service
            .git_version(&CancellationToken::new())
            .await
            .unwrap();
    }
    assert!(!service.process_observations().complete());
    drop(service);

    let reopened = GitService::new(&git, &state).unwrap();
    let snapshot = reopened.process_observations();
    assert!(!snapshot.complete());
    assert!(snapshot.omitted() > 0 || snapshot.omitted_bytes() > 0);
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
