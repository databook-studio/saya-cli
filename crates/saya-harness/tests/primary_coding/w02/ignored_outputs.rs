use std::fs;

use saya_agent::CancellationToken;
use saya_harness::coding::git::{GitInventoryError, GitService};

use super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[tokio::test]
async fn large_ignored_output_tree_is_reported_compactly_without_recursive_inventory() {
    let temp = TempRoot::new("ignored-large");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::write(root.join(".gitignore"), b"ignored-output/\n").unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", ".gitignore"]));
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "ignore build output"]),
    );
    let output = root.join("ignored-output");
    fs::create_dir(&output).unwrap();
    for index in 0..20_100 {
        fs::write(output.join(format!("artifact-{index}")), b"ignored").unwrap();
    }

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let inventory = service
        .inventory_repository(&opened, &cancellation)
        .await
        .unwrap_or_else(|error| panic!("ignored output is excluded from nested scan: {error}"));

    assert!(
        inventory.paths().iter().any(|path| {
            path.ignored() && path.path() == std::path::Path::new("ignored-output")
        })
    );
    assert!(!inventory.paths().iter().any(|path| {
        path.path()
            .components()
            .any(|component| component.as_os_str() == "ignored-output")
            && path.path() != std::path::Path::new("ignored-output")
    }));
}

#[tokio::test]
async fn tracked_descendant_keeps_ignored_ancestors_in_nested_repository_scan() {
    let temp = TempRoot::new("ignored-tracked-parent");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let ignored = root.join("ignored-prefix");
    fs::create_dir(&ignored).unwrap();
    fs::write(root.join(".gitignore"), b"ignored-prefix/\n").unwrap();
    fs::write(
        ignored.join("tracked.txt"),
        b"tracked beneath ignored prefix\n",
    )
    .unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", ".gitignore"]));
    run_git(
        &git,
        temp.path(),
        git_args(
            &root,
            &["add", "--force", "--", "ignored-prefix/tracked.txt"],
        ),
    );
    run_git(
        &git,
        temp.path(),
        git_args(
            &root,
            &["commit", "--quiet", "-m", "track ignored descendant"],
        ),
    );
    let template = format!(
        "--template={}",
        temp.path().join("empty-template").display()
    );
    run_git(
        &git,
        temp.path(),
        git_args(&ignored, &["init", "--quiet", &template]),
    );

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::NestedRepository)
    ));
}

#[tokio::test]
async fn untracked_directory_token_checks_its_own_nested_git_marker() {
    let temp = TempRoot::new("untracked-nested");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let nested = root.join("new-untracked-directory");
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

    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    assert!(matches!(
        service.inventory_repository(&opened, &cancellation).await,
        Err(GitInventoryError::NestedRepository)
    ));
}
