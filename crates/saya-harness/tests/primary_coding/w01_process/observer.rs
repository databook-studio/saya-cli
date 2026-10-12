use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitProcessEffect, GitProcessOutcome, GitService};

use crate::w01::{
    state_root,
    support::{TempRoot, fake_git},
};

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
    let snapshot = service.process_observations();
    let receipt = snapshot.receipts().last().unwrap();
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
