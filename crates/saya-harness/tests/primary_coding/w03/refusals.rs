use std::{fs, process::Command};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{SnapshotError, SnapshotOmissionPolicy, SnapshotOmissionReason, SnapshotSelection},
};
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[tokio::test]
async fn tracked_sensitive_source_refuses_instead_of_being_silently_omitted() {
    let temp = TempRoot::new("snapshot-sensitive-tracked");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join(".env.production"), b"fixture placeholder only\n").unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", ".env.production"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "tracked env path"]),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-sensitive-tracked").unwrap(),
                SnapshotSelection::Head,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::SensitiveTrackedPath)
    ));
}

#[tokio::test]
async fn untracked_sensitive_and_explicit_private_paths_are_omitted_with_reasons() {
    let temp = TempRoot::new("snapshot-sensitive-untracked");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join("id_rsa"), b"placeholder key material\n").unwrap();
    fs::create_dir(root.join("private")).unwrap();
    fs::write(root.join("private/local.txt"), b"private fixture\n").unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, vec!["private".to_owned()]).unwrap();
    let manifest = service
        .plan_snapshot(
            &repository,
            SnapshotId::parse("snapshot-sensitive-untracked").unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();

    assert!(
        !manifest
            .entries()
            .iter()
            .any(|entry| entry.path().as_str() == "id_rsa"
                || entry.path().as_str().starts_with("private/"))
    );
    assert!(manifest.omissions().iter().any(|fact| {
        fact.path().as_str() == "id_rsa"
            && fact.reason() == SnapshotOmissionReason::SensitiveUntracked
    }));
    assert!(manifest.omissions().iter().any(|fact| {
        fact.path().as_str() == "private/local.txt"
            && fact.reason() == SnapshotOmissionReason::ConfiguredPrivatePath
    }));
}

#[tokio::test]
async fn conflicted_index_refuses_snapshot_selection() {
    let temp = TempRoot::new("snapshot-conflict");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let branch = run_git(
        &git,
        temp.path(),
        git_args(&root, &["branch", "--show-current"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["switch", "-c", "snapshot-conflict"]),
    );
    fs::write(root.join("tracked.txt"), b"side branch\n").unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", "tracked.txt"]));
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "side change"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(
            &root,
            &["switch", std::str::from_utf8(&branch).unwrap().trim()],
        ),
    );
    fs::write(root.join("tracked.txt"), b"main branch\n").unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", "tracked.txt"]));
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "main change"]),
    );
    let merge = Command::new(&git)
        .env_clear()
        .env("PATH", git.parent().unwrap())
        .env("HOME", temp.path().join("git-home"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            temp.path().join("git-home/empty.gitconfig"),
        )
        .args(git_args(&root, &["merge", "snapshot-conflict"]))
        .output()
        .unwrap();
    assert!(!merge.status.success(), "the fixture merge must conflict");

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-conflict").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::ConflictedIndex)
    ));
}

#[tokio::test]
async fn private_path_policy_cannot_be_reused_for_another_repository() {
    let temp = TempRoot::new("snapshot-policy-binding");
    let git = git_executable();
    let first = repository(temp.path(), &git);
    let second_parent = temp.path().join("second");
    fs::create_dir(&second_parent).unwrap();
    let second = repository(&second_parent, &git);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let first_repository = service
        .open_repository(&first, &cancellation)
        .await
        .unwrap();
    let second_repository = service
        .open_repository(&second, &cancellation)
        .await
        .unwrap();
    let policy =
        SnapshotOmissionPolicy::new(&first_repository, vec!["private".to_owned()]).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &second_repository,
                SnapshotId::parse("snapshot-policy-mismatch").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::PolicyRepositoryMismatch)
    ));
}

#[tokio::test]
async fn symlink_and_case_insensitive_git_paths_refuse_selection() {
    let temp = TempRoot::new("snapshot-unsafe-path");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    #[cfg(unix)]
    std::os::unix::fs::symlink(temp.path().join("outside"), root.join("link")).unwrap();
    fs::write(temp.path().join("outside"), b"outside fixture\n").unwrap();

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let repository = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-symlink").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::UnsafePath)
    ));

    fs::remove_file(root.join("link")).unwrap();
    let nested = root.join("nested");
    fs::create_dir(&nested).unwrap();
    let template = format!(
        "--template={}",
        temp.path().join("empty-template").display()
    );
    run_git(
        &git,
        temp.path(),
        git_args(&nested, &["init", "--quiet", &template]),
    );
    fs::rename(nested.join(".git"), nested.join(".GIT")).unwrap();
    let policy = SnapshotOmissionPolicy::new(&repository, Vec::new()).unwrap();
    assert!(matches!(
        service
            .plan_snapshot(
                &repository,
                SnapshotId::parse("snapshot-uppercase-git").unwrap(),
                SnapshotSelection::CurrentWorkingContent,
                policy,
                &cancellation,
            )
            .await,
        Err(SnapshotError::GitInventory(
            saya_harness::coding::git::GitInventoryError::NestedRepository
        ))
    ));
}
