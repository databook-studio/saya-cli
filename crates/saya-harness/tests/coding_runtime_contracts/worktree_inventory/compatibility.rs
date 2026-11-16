use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitConfigRisk, GitError, GitService};

use super::support::{
    TempRoot, git_args, git_executable, repository, run_git, state_root, versioned_git,
};

#[tokio::test]
async fn old_git_never_receives_the_no_lazy_fetch_option_or_environment() {
    let temp = TempRoot::new("old-git-compatibility");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    let (wrapper, log) = versioned_git(temp.path(), &real_git, "2.34.1", true);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    service
        .inventory_repository(&opened, &cancellation)
        .await
        .unwrap();
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(!recorded.contains("flag=yes"), "{recorded}");
    assert!(!recorded.contains("env=1"), "{recorded}");
    assert!(!recorded.contains("worktree_list_z=yes"), "{recorded}");
}

#[tokio::test]
async fn modern_git_receives_the_option_and_zero_terminated_listing() {
    let temp = TempRoot::new("modern-git-compatibility");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    let (wrapper, log) = versioned_git(temp.path(), &real_git, "2.50.1", false);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    service
        .inventory_repository(&opened, &cancellation)
        .await
        .unwrap();
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(recorded.contains("flag=yes"), "{recorded}");
    assert!(
        !recorded.contains("env=1"),
        "the option protects the call; no environment variable is set: {recorded}"
    );
    assert!(recorded.contains("worktree_list_z=yes"), "{recorded}");
}

#[tokio::test]
async fn a_git_below_the_supported_minimum_is_refused_at_the_probe() {
    let temp = TempRoot::new("too-old-git");
    let real_git = git_executable();
    let (wrapper, _) = versioned_git(temp.path(), &real_git, "2.33.1", true);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let error = service
        .git_compatibility(&CancellationToken::new())
        .await
        .expect_err("2.33 is below the supported minimum");
    assert!(
        matches!(error, GitError::UnsupportedGitVersionTooOld { .. }),
        "{error}"
    );
}

#[tokio::test]
async fn old_git_refuses_only_partial_clone_repositories() {
    let temp = TempRoot::new("old-git-partial-clone");
    let real_git = git_executable();
    let root = repository(temp.path(), &real_git);
    run_git(
        &real_git,
        temp.path(),
        git_args(&root, &["config", "extensions.partialClone", "origin"]),
    );
    let (wrapper, _) = versioned_git(temp.path(), &real_git, "2.34.1", true);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    assert!(matches!(
        service
            .open_repository(&root, &CancellationToken::new())
            .await,
        Err(GitError::UnsupportedConfiguration(
            GitConfigRisk::PartialClone
        ))
    ));
}

#[tokio::test]
async fn old_git_compatibility_is_detected_once_and_cached() {
    let temp = TempRoot::new("old-git-cached");
    let real_git = git_executable();
    let (wrapper, log) = versioned_git(temp.path(), &real_git, "2.34.1", true);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let first = service.git_compatibility(&cancellation).await.unwrap();
    let second = service.git_compatibility(&cancellation).await.unwrap();
    assert_eq!(first, second);
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert_eq!(recorded.matches("arg=--version").count(), 1, "{recorded}");
}

#[tokio::test]
async fn an_unparseable_version_is_treated_as_the_minimum_supported() {
    let temp = TempRoot::new("unknown-git-version");
    let real_git = git_executable();
    let (wrapper, _) = versioned_git(temp.path(), &real_git, "unknown", true);
    let service = GitService::new(&wrapper, state_root(temp.path())).unwrap();
    let compatibility = service
        .git_compatibility(&CancellationToken::new())
        .await
        .unwrap();
    assert!(!compatibility.version_known());
    assert_eq!(
        compatibility.version(),
        saya_harness::coding::git::MINIMUM_SUPPORTED
    );
    assert_eq!(
        compatibility.lazy_fetch_protection(),
        saya_harness::coding::git::LazyFetchProtection::RefusePartialClone
    );
}
