//! Public-boundary contracts for ordinary repository symlinks: tracked and
//! untracked relative targets inside the repository are snapshotted as
//! symlinks, materialized as symlinks, and carried through the live walk
//! unchanged; absolute, escaping, and `.git` targets refuse with a message
//! that names the path and the reason.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_harness::{
    coding::{
        git::{GitService, IgnoredPaths},
        integration::{DEFAULT_DELTA_DEADLINE, candidate_source_digest, compute_delta_at},
        snapshot::{
            CapturedSnapshot, SnapshotArtifacts, SnapshotError, SnapshotManifest,
            SnapshotOmissionPolicy, SnapshotSelection, materialize_snapshot,
        },
    },
    workspace::Workspace,
};
use saya_types::SnapshotId;

use support::{TempRoot, git_args, git_executable, repository, run_git, state_root};

#[path = "snapshot_symlinks/support.rs"]
mod support;

fn artifacts_root(temp: &TempRoot) -> PathBuf {
    temp.path().join("artifacts")
}

/// The job state root is created once per temp root; planning and capturing
/// share it.
fn shared_state_root(temp: &TempRoot) -> PathBuf {
    let root = temp.path().join("application-state");
    if root.exists() {
        root
    } else {
        state_root(temp.path())
    }
}

fn empty_private_target(parent: &Path, name: &str) -> PathBuf {
    let target = parent.join(name);
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    target
}

async fn plan(
    temp: &TempRoot,
    root: &Path,
    selection: SnapshotSelection,
    label: &str,
) -> Result<SnapshotManifest, SnapshotError> {
    let git = git_executable();
    let service = GitService::new(&git, shared_state_root(temp)).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(label).unwrap(),
            selection,
            policy,
            &cancellation,
        )
        .await
}

async fn capture(
    temp: &TempRoot,
    root: &Path,
    manifest: &SnapshotManifest,
) -> (SnapshotArtifacts, CapturedSnapshot) {
    let git = git_executable();
    let service = GitService::new(&git, shared_state_root(temp)).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(root, &cancellation).await.unwrap();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = service
        .capture_snapshot(&opened, &workspace, manifest, &artifacts, &cancellation)
        .await
        .unwrap();
    (artifacts, captured)
}

fn assert_materialized_symlink(target: &Path, path: &str, expected: &str) {
    let metadata = fs::symlink_metadata(target.join(path)).unwrap();
    assert!(
        metadata.file_type().is_symlink(),
        "{path} must materialize as a symlink"
    );
    assert_eq!(
        fs::read_link(target.join(path)).unwrap(),
        Path::new(expected)
    );
}

async fn assert_unchanged_delta(
    target: &Path,
    captured: &CapturedSnapshot,
    cancellation: &CancellationToken,
) {
    let delta = compute_delta_at(
        target,
        captured.snapshot_id(),
        captured.captured_manifest_sha256(),
        captured,
        cancellation,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
    assert!(
        delta.is_empty(),
        "an unchanged symlink tree must produce no delta: {:?}",
        delta
    );
    candidate_source_digest(
        target,
        &IgnoredPaths::none(),
        cancellation,
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap();
}

#[tokio::test]
async fn tracked_relative_symlinks_snapshot_materialize_and_hold() {
    let temp = TempRoot::new("snapshot-symlink-tracked");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::create_dir(root.join("dir")).unwrap();
    fs::write(root.join("dir/file.txt"), b"dir bytes\n").unwrap();
    symlink("tracked.txt", root.join("link-file")).unwrap();
    symlink("dir", root.join("link-dir")).unwrap();
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["add", "dir", "link-file", "link-dir"]),
    );
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "tracked symlinks"]),
    );

    let manifest = plan(
        &temp,
        &root,
        SnapshotSelection::Head,
        "snapshot-symlink-tracked",
    )
    .await
    .unwrap();
    for (path, target) in [("link-file", "tracked.txt"), ("link-dir", "dir")] {
        let entry = manifest
            .entries()
            .iter()
            .find(|entry| entry.path().as_str() == path)
            .unwrap_or_else(|| panic!("{path} must be in the plan"));
        assert_eq!(entry.declared_bytes(), target.len() as u64);
    }

    let (artifacts, captured) = capture(&temp, &root, &manifest).await;
    let target = empty_private_target(temp.path(), "materialized");
    let cancellation = CancellationToken::new();
    materialize_snapshot(&artifacts, manifest.snapshot_id(), &target, &cancellation).unwrap();
    assert_materialized_symlink(&target, "link-file", "tracked.txt");
    assert_materialized_symlink(&target, "link-dir", "dir");
    assert_eq!(
        fs::read(target.join("dir/file.txt")).unwrap(),
        b"dir bytes\n"
    );
    assert_eq!(fs::read(target.join("tracked.txt")).unwrap(), b"base\n");
    assert_unchanged_delta(&target, &captured, &cancellation).await;

    // Apply-back only touches delta paths: the source repository's symlinks
    // are still symlinks with the same targets after integration.
    assert_materialized_symlink(&root, "link-file", "tracked.txt");
    assert_materialized_symlink(&root, "link-dir", "dir");
}

#[tokio::test]
async fn worktree_relative_symlinks_snapshot_and_materialize() {
    let temp = TempRoot::new("snapshot-symlink-worktree");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::create_dir(root.join("newdir")).unwrap();
    fs::write(root.join("newdir/inside.txt"), b"inside bytes\n").unwrap();
    fs::write(root.join("new.txt"), b"new bytes\n").unwrap();
    symlink("new.txt", root.join("work-link")).unwrap();
    symlink("newdir", root.join("work-dir-link")).unwrap();

    let manifest = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "snapshot-symlink-worktree",
    )
    .await
    .unwrap();
    for path in ["work-link", "work-dir-link"] {
        assert!(
            manifest
                .entries()
                .iter()
                .any(|entry| entry.path().as_str() == path),
            "{path} must be in the plan"
        );
    }

    let (artifacts, captured) = capture(&temp, &root, &manifest).await;
    let target = empty_private_target(temp.path(), "materialized");
    let cancellation = CancellationToken::new();
    materialize_snapshot(&artifacts, manifest.snapshot_id(), &target, &cancellation).unwrap();
    assert_materialized_symlink(&target, "work-link", "new.txt");
    assert_materialized_symlink(&target, "work-dir-link", "newdir");
    assert_unchanged_delta(&target, &captured, &cancellation).await;
}

#[tokio::test]
async fn chained_escaping_symlink_refuses_at_selection() {
    let temp = TempRoot::new("snapshot-symlink-chain");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    fs::create_dir(root.join("d")).unwrap();
    // `d/up` resolves to the root; `d/esc` then rises two more levels, so
    // the whole chain leaves the repository even though `d/esc` alone
    // resolves lexically to the root.
    symlink("..", root.join("d/up")).unwrap();
    symlink("up/../..", root.join("d/esc")).unwrap();
    let error = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "snapshot-symlink-chain",
    )
    .await
    .unwrap_err();
    assert_unsafe_symlink(error, "d/esc", "escapes");
}

#[tokio::test]
async fn in_root_two_link_chain_is_accepted() {
    let temp = TempRoot::new("snapshot-symlink-in-root-chain");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    symlink("tracked.txt", root.join("first")).unwrap();
    symlink("first", root.join("second")).unwrap();

    let manifest = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "snapshot-symlink-in-root-chain",
    )
    .await
    .unwrap();
    for path in ["first", "second"] {
        assert!(
            manifest
                .entries()
                .iter()
                .any(|entry| entry.path().as_str() == path),
            "{path} must be in the plan"
        );
    }
    let (artifacts, captured) = capture(&temp, &root, &manifest).await;
    let target = empty_private_target(temp.path(), "materialized");
    let cancellation = CancellationToken::new();
    materialize_snapshot(&artifacts, manifest.snapshot_id(), &target, &cancellation).unwrap();
    assert_materialized_symlink(&target, "first", "tracked.txt");
    assert_materialized_symlink(&target, "second", "first");
    assert_unchanged_delta(&target, &captured, &cancellation).await;
}

#[tokio::test]
async fn symlink_loop_refuses() {
    let temp = TempRoot::new("snapshot-symlink-loop");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    symlink("b", root.join("a")).unwrap();
    symlink("a", root.join("b")).unwrap();
    let error = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "snapshot-symlink-loop",
    )
    .await
    .unwrap_err();
    assert_unsafe_symlink(error, "a", "loop or chain that is too long");
}

#[tokio::test]
async fn chain_over_the_hop_bound_refuses() {
    let temp = TempRoot::new("snapshot-symlink-hop-bound");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    for index in 0..50 {
        symlink(format!("l{}", index + 1), root.join(format!("l{index}"))).unwrap();
    }
    symlink("tracked.txt", root.join("l50")).unwrap();
    let error = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "snapshot-symlink-hop-bound",
    )
    .await
    .unwrap_err();
    assert_unsafe_symlink(error, "l0", "loop or chain that is too long");
}

fn assert_unsafe_symlink(error: SnapshotError, path: &str, reason: &str) {
    assert!(
        matches!(error, SnapshotError::UnsafePath { .. }),
        "expected UnsafePath, got {error:?}"
    );
    let message = error.to_string();
    assert!(
        message.contains(path),
        "the refusal must name `{path}`: {message}"
    );
    assert!(
        message.contains(reason),
        "the refusal must say `{reason}`: {message}"
    );
}

#[tokio::test]
async fn absolute_escaping_and_git_symlinks_refuse_with_named_reason() {
    // Untracked (worktree) symlinks.
    for (path, target, reason) in [
        ("abs-link", "/tmp/saya-outside", "absolute"),
        ("escape-link", "../outside", "escapes"),
        ("git-link", ".git/config", ".git"),
    ] {
        let temp = TempRoot::new("snapshot-symlink-refusal");
        let git = git_executable();
        let root = repository(temp.path(), &git);
        symlink(target, root.join(path)).unwrap();
        let error = plan(
            &temp,
            &root,
            SnapshotSelection::CurrentWorkingContent,
            "snapshot-symlink-refusal",
        )
        .await
        .unwrap_err();
        assert_unsafe_symlink(error, path, reason);
    }

    // A tracked symlink with an absolute target is refused from the HEAD
    // tree, not only from the worktree scan.
    let temp = TempRoot::new("snapshot-symlink-tracked-absolute");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    symlink("/tmp/saya-outside", root.join("abs-link")).unwrap();
    run_git(&git, temp.path(), git_args(&root, &["add", "abs-link"]));
    run_git(
        &git,
        temp.path(),
        git_args(&root, &["commit", "--quiet", "-m", "absolute symlink"]),
    );
    let error = plan(
        &temp,
        &root,
        SnapshotSelection::Head,
        "snapshot-symlink-tracked-absolute",
    )
    .await
    .unwrap_err();
    assert_unsafe_symlink(error, "abs-link", "absolute");
}
