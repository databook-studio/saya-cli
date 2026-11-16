//! Shell-made changes in an in-place root: a local filter config is never run,
//! a status read never writes the index, and pre-existing or tool-touched
//! paths are not the job's changes.
#![cfg(unix)]

use std::{collections::HashSet, fs, path::Path, time::Duration};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    local::shell_changes::{
        ShellChange, ShellChangeKind, ShellChanges, ShellStart, ShellUnrecorded, capture_start,
        shell_changes,
    },
};

use support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[path = "snapshot_symlinks/support.rs"]
mod support;

const DEADLINE: Duration = Duration::from_secs(10);

fn service(temp: &TempRoot) -> GitService {
    GitService::new(git_executable(), state_root(temp.path())).unwrap()
}

fn record_index(root: &Path) -> (u64, std::time::SystemTime) {
    let metadata = fs::metadata(root.join(".git/index")).unwrap();
    (metadata.len(), metadata.modified().unwrap())
}

#[tokio::test]
async fn local_filter_config_is_unrecorded() {
    let temp = TempRoot::new("shell-filter");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["config", "filter.x.clean", "false"]),
    );

    let service = service(&temp);
    let start = capture_start(&service, &root, &CancellationToken::new(), DEADLINE).await;
    assert_eq!(
        start,
        ShellStart::Unrecorded {
            reason: ShellUnrecorded::GitConfigFilters
        },
        "a repository filter would execute a repository-chosen program"
    );
}

#[tokio::test]
async fn status_does_not_write_index() {
    let temp = TempRoot::new("shell-index");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let service = service(&temp);
    let start = capture_start(&service, &root, &CancellationToken::new(), DEADLINE).await;
    assert!(
        matches!(start, ShellStart::Recorded { .. }),
        "the fixture is a clean repository: {start:?}"
    );

    let before = record_index(&root);
    fs::write(root.join("tracked.txt"), b"changed by the host shell\n").unwrap();
    let changes = shell_changes(
        &service,
        &root,
        &start,
        &HashSet::new(),
        &CancellationToken::new(),
    )
    .await;
    match changes {
        ShellChanges::Recorded { paths, .. } => assert_eq!(
            paths,
            vec![ShellChange {
                path: "tracked.txt".to_owned(),
                kind: ShellChangeKind::Modified,
            }]
        ),
        other => panic!("the shell edit is recorded: {other:?}"),
    }
    assert_eq!(
        record_index(&root),
        before,
        "a review never takes or rewrites the index"
    );
}

#[tokio::test]
async fn pre_existing_and_touched_paths_are_excluded() {
    let temp = TempRoot::new("shell-exclusions");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    // Dirty before the job started.
    fs::write(root.join("tracked.txt"), b"pre-existing dirty\n").unwrap();
    let service = service(&temp);
    let start = capture_start(&service, &root, &CancellationToken::new(), DEADLINE).await;
    assert!(matches!(start, ShellStart::Recorded { .. }), "{start:?}");

    // One shell edit and one tool edit after start.
    fs::write(root.join("shell.txt"), b"host\n").unwrap();
    fs::write(root.join("tool.txt"), b"tool\n").unwrap();
    let touched = HashSet::from(["tool.txt".to_owned()]);
    let changes = shell_changes(&service, &root, &start, &touched, &CancellationToken::new()).await;
    match changes {
        ShellChanges::Recorded { paths, .. } => {
            assert_eq!(
                paths,
                vec![ShellChange {
                    path: "shell.txt".to_owned(),
                    kind: ShellChangeKind::Added,
                }],
                "the pre-existing dirty file and the tool edit are not shell changes"
            );
        }
        other => panic!("the shell edit is recorded: {other:?}"),
    }
}

#[tokio::test]
async fn a_plain_folder_is_unrecorded() {
    let temp = TempRoot::new("shell-plain");
    let root = temp.path().join("plain");
    fs::create_dir(&root).unwrap();
    let service = service(&temp);
    let start = capture_start(&service, &root, &CancellationToken::new(), DEADLINE).await;
    assert_eq!(
        start,
        ShellStart::Unrecorded {
            reason: ShellUnrecorded::NotAGitRepository
        }
    );
    let changes = shell_changes(
        &service,
        &root,
        &start,
        &HashSet::new(),
        &CancellationToken::new(),
    )
    .await;
    assert_eq!(
        changes,
        ShellChanges::Unrecorded {
            reason: ShellUnrecorded::NotAGitRepository
        }
    );
}
