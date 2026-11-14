use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{SnapshotContentSource, SnapshotOmissionPolicy, SnapshotSelection},
};
use saya_types::{GitObjectId, SnapshotId};

use super::super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[tokio::test]
async fn dirty_snapshot_keeps_index_and_untracked_intent_without_mutating_source() {
    let temp = TempRoot::new("snapshot-dirty");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"next staged bytes\n").unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", "tracked.txt"]));
    fs::write(root.join("tracked.txt"), b"working bytes\n").unwrap();
    fs::write(root.join("new.txt"), b"untracked bytes\n").unwrap();
    fs::write(root.join(".gitignore"), b"ignored-output/\n").unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", ".gitignore"]));
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "ignore output"]),
    );
    fs::write(root.join("tracked.txt"), b"staged bytes\n").unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", "tracked.txt"]));
    fs::write(root.join("tracked.txt"), b"working bytes\n").unwrap();
    let ignored = root.join("ignored-output");
    fs::create_dir(&ignored).unwrap();
    fs::write(ignored.join("artifact"), b"not selected").unwrap();
    let before_head = run_git(&git, temp.path(), git_args(&root, &["rev-parse", "HEAD"]));
    let before_index = fs::read(root.join(".git/index")).unwrap();
    let before_working = fs::read(root.join("tracked.txt")).unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-dirty").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    let tracked = manifest
        .entries()
        .iter()
        .find(|entry| entry.path().as_str() == "tracked.txt")
        .unwrap();
    assert_eq!(tracked.source(), &SnapshotContentSource::WorkingTree);
    assert_eq!(tracked.declared_bytes(), b"working bytes\n".len() as u64);
    assert!(
        manifest
            .index_entries()
            .iter()
            .any(|entry| { entry.path().as_str() == "tracked.txt" && entry.stage() == 0 })
    );
    assert!(
        manifest.status().iter().any(|fact| {
            fact.path().as_str() == "tracked.txt" && fact.staged() && fact.unstaged()
        })
    );
    assert!(manifest.entries().iter().any(|entry| {
        entry.path().as_str() == "new.txt" && entry.source() == &SnapshotContentSource::WorkingTree
    }));
    assert!(
        !manifest
            .entries()
            .iter()
            .any(|entry| entry.path().as_str().starts_with("ignored-output/"))
    );
    assert!(
        manifest
            .omissions()
            .iter()
            .any(|fact| { fact.path().as_str() == "ignored-output" && fact.reason().is_ignored() })
    );
    assert_eq!(
        before_head,
        run_git(&git, temp.path(), git_args(&root, &["rev-parse", "HEAD"]))
    );
    assert_eq!(before_index, fs::read(root.join(".git/index")).unwrap());
    assert_eq!(before_working, fs::read(root.join("tracked.txt")).unwrap());
    assert_eq!(
        fs::read(root.join("new.txt")).unwrap(),
        b"untracked bytes\n"
    );
}

#[tokio::test]
async fn head_snapshot_uses_commit_blobs_and_ignores_dirty_overlay() {
    let temp = TempRoot::new("snapshot-head");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("tracked.txt"), b"dirty overlay\n").unwrap();
    fs::write(root.join("untracked.txt"), b"not in HEAD\n").unwrap();
    let expected = run_git(
        &git,
        temp.path(),
        git_args(&root, &["rev-parse", "HEAD:tracked.txt"]),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-head").unwrap(),
            SnapshotSelection::Head,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    let tracked = manifest
        .entries()
        .iter()
        .find(|entry| entry.path().as_str() == "tracked.txt")
        .unwrap();
    assert_eq!(
        tracked.source(),
        &SnapshotContentSource::GitBlob {
            object_id: saya_types::GitObjectId::parse(
                std::str::from_utf8(&expected).unwrap().trim(),
            )
            .unwrap(),
        }
    );
    assert!(
        !manifest
            .entries()
            .iter()
            .any(|entry| entry.path().as_str() == "untracked.txt")
    );
    assert_eq!(manifest.selection(), SnapshotSelection::Head);
}

#[tokio::test]
async fn staged_rename_delete_and_intent_to_add_remain_separate_facts() {
    let temp = TempRoot::new("snapshot-index-facts");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("deleted.txt"), b"delete me\n").unwrap();
    fs::write(root.join("staged-empty.txt"), b"committed bytes\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "deleted.txt", "staged-empty.txt"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "second tracked path"]),
    );
    fs::write(root.join("staged-empty.txt"), b"").unwrap();
    let empty_blob = run_git(
        &git,
        temp.path(),
        git_args(&root, &["hash-object", "--", "staged-empty.txt"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "staged-empty.txt"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["mv", "tracked.txt", "renamed.txt"]),
    );
    run_git(&git, temp.path(), git_args(&root, &["rm", "deleted.txt"]));
    fs::write(root.join("intent.txt"), b"intent bytes\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "-N", "--", "intent.txt"]),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-index-facts").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    assert!(
        manifest
            .changes()
            .iter()
            .any(|change| { change.is_rename("tracked.txt", "renamed.txt") })
    );
    assert!(
        manifest
            .changes()
            .iter()
            .any(|change| change.is_tombstone("deleted.txt"))
    );
    assert!(
        manifest
            .index_entries()
            .iter()
            .any(|entry| { entry.path().as_str() == "intent.txt" && entry.intent_to_add() })
    );
    let staged_empty = manifest
        .index_entries()
        .iter()
        .find(|entry| entry.path().as_str() == "staged-empty.txt")
        .unwrap();
    assert!(!staged_empty.intent_to_add());
    assert_eq!(
        staged_empty.object_id(),
        &GitObjectId::parse(std::str::from_utf8(&empty_blob).unwrap().trim()).unwrap()
    );
    assert!(manifest.status().iter().any(|fact| {
        fact.path().as_str() == "staged-empty.txt" && fact.staged() && !fact.unstaged()
    }));
}
