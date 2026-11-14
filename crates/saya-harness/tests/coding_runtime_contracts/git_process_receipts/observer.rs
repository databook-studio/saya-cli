use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitError, GitProcessEffect, GitProcessOutcome, GitService};

use crate::git_processes::{
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

#[tokio::test]
async fn a_git_older_than_the_minimum_is_refused_by_the_first_probe() {
    let temp = TempRoot::new("git-too-old");
    let (git, _) = fake_git(temp.path(), "#!/bin/sh\nprintf 'git version 2.33.1\\n'\n");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let error = service
        .git_version(&CancellationToken::new())
        .await
        .expect_err("Git 2.33 is below the supported minimum");
    let message = error.to_string();
    assert!(message.contains("2.33.1"), "{message}");
    assert!(message.contains("2.34.0"), "{message}");
}

#[tokio::test]
async fn the_first_real_call_probes_first_and_carries_no_lazy_fetch() {
    let temp = TempRoot::new("probe-first");
    let script = "#!/bin/sh\n\
        version=no\n\
        for arg in \"$@\"; do\n\
          if [ \"$arg\" = \"--version\" ]; then version=yes; fi\n\
        done\n\
        {\n\
          if [ \"$version\" = yes ]; then printf 'version'; else printf 'call'; fi\n\
          for arg in \"$@\"; do printf ' %s' \"$arg\"; done\n\
          printf '\\n'\n\
        } >> \"$0.argv\"\n\
        if [ \"$version\" = yes ]; then printf 'git version 2.50.1\\n'; fi\n\
        exit 0\n";
    let (git, _) = fake_git(temp.path(), script);
    let log = git.with_extension("argv");
    let state = state_root(temp.path());
    let service = GitService::new(&git, &state).unwrap();
    let repo = temp.path().join("repo");
    std::fs::create_dir(&repo).unwrap();
    // The fake prints nothing for a real request, so the repository check
    // fails after the call; the recorded argv is what this test reads.
    let _ = service
        .open_repository(&repo, &CancellationToken::new())
        .await;
    let recorded = std::fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = recorded.lines().collect();
    assert_eq!(
        lines.first().map(|line| line.split(' ').next()),
        Some(Some("version")),
        "the version probe runs first: {recorded}"
    );
    let first_call = lines
        .iter()
        .find(|line| line.starts_with("call "))
        .expect("a real call follows the probe");
    assert!(
        first_call.contains("--no-lazy-fetch"),
        "the first real call carries the global option: {first_call}"
    );
}
