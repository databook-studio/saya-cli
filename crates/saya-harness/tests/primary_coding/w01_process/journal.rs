use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::git::GitService;

use crate::w01::{
    state_root,
    support::{TempRoot, fake_git},
};

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
