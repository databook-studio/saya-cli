//! Case- and normalization-variant symlink chains. The resolver must treat a
//! component that names a link only after case folding (and one Unicode
//! normal form) as that link, and must refuse a `..` that follows such a
//! component, because the managed checkout runs on case-insensitive
//! filesystems. Selection, materialization, and the live delta walk each
//! refuse the audit's `a/b/d/up` + `a/b/d/esc` pair.
#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_harness::{
    coding::{
        git::GitService,
        integration::{DEFAULT_DELTA_DEADLINE, DeltaError, compute_delta_at},
        snapshot::{
            CapturedSnapshot, SnapshotArtifacts, SnapshotError, SnapshotManifest,
            SnapshotOmissionPolicy, SnapshotSelection, materialize_snapshot,
        },
    },
    workspace::Workspace,
};
use saya_types::{Sha256Digest, SnapshotId};

use support::{TempRoot, git_executable, repository, state_root};

#[path = "snapshot_symlinks/support.rs"]
mod support;

fn artifacts_root(temp: &TempRoot) -> PathBuf {
    temp.path().join("artifacts")
}

fn shared_state_root(temp: &TempRoot) -> PathBuf {
    let root = temp.path().join("application-state");
    if root.exists() {
        root
    } else {
        state_root(temp.path())
    }
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

fn empty_private_target(parent: &Path, name: &str) -> PathBuf {
    let target = parent.join(name);
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    target
}

/// The audit's pair: `a/b/d/up` resolves to the root, and `a/b/d/esc` names
/// it as `UP` then climbs three levels above the root.
fn plant_case_variant_chain(root: &Path) {
    fs::create_dir_all(root.join("a/b/d")).unwrap();
    symlink("../../..", root.join("a/b/d/up")).unwrap();
    symlink("UP/../../../x", root.join("a/b/d/esc")).unwrap();
}

/// The same class through Unicode: the link is stored composed (`Café`) and
/// the target names it decomposed and upper-case (`CAFE` + combining acute).
fn plant_unicode_variant_chain(root: &Path) {
    fs::create_dir(root.join("d")).unwrap();
    symlink("..", root.join("d/Caf\u{e9}")).unwrap();
    symlink("CAFE\u{301}/../../x", root.join("d/esc")).unwrap();
}

fn assert_refused(error: SnapshotError, path: &str) {
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
        message.contains("case variant"),
        "the refusal must state the folded-component reason: {message}"
    );
}

/// An independent SHA-256 over the exact bytes.
fn digest(bytes: &[u8]) -> Sha256Digest {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in Sha256::digest(bytes) {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    Sha256Digest::parse(&hex).unwrap()
}

fn rewrite_manifest(dir: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let manifest_path = dir.join("manifest.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    edit(&mut value);
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&manifest_path, &bytes).unwrap();
    fs::write(dir.join("COMPLETE"), digest(&bytes).as_str()).unwrap();
}

#[tokio::test]
async fn case_variant_chain_refuses_at_selection() {
    let temp = TempRoot::new("casefold-select");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    plant_case_variant_chain(&root);
    let error = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "casefold-select",
    )
    .await
    .unwrap_err();
    assert_refused(error, "a/b/d/esc");
}

#[tokio::test]
async fn unicode_variant_chain_refuses_at_selection() {
    let temp = TempRoot::new("casefold-unicode");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    plant_unicode_variant_chain(&root);
    let error = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "casefold-unicode",
    )
    .await
    .unwrap_err();
    assert_refused(error, "d/esc");
}

#[tokio::test]
async fn case_variant_chain_refuses_at_materialize() {
    let temp = TempRoot::new("casefold-materialize");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let manifest = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "casefold-materialize",
    )
    .await
    .unwrap();
    let (artifacts, _captured) = capture(&temp, &root, &manifest).await;
    // The audit's pair cannot be published through the planner after the fix,
    // so the stored manifest is rewritten directly and the objects the links
    // name are written by hand.
    let dir = artifacts_root(&temp).join(manifest.snapshot_id().as_str());
    let objects = dir.join("objects");
    fs::create_dir_all(&objects).unwrap();
    rewrite_manifest(&dir, |value| {
        let entries = value["entries"].as_array_mut().unwrap();
        entries.clear();
        for (path, target) in [("a/b/d/up", "../../.."), ("a/b/d/esc", "UP/../../../x")] {
            let sha = digest(target.as_bytes());
            fs::write(objects.join(sha.as_str()), target.as_bytes()).unwrap();
            entries.push(serde_json::json!({
                "path": path,
                "mode": "symlink",
                "content_sha256": sha.as_str(),
                "captured_bytes": target.len(),
                "source": {"kind": "working_tree"},
            }));
        }
    });
    let target = empty_private_target(temp.path(), "materialized");
    let error = materialize_snapshot(
        &artifacts,
        manifest.snapshot_id(),
        &target,
        &CancellationToken::new(),
    )
    .unwrap_err();
    assert_refused(error, "a/b/d/esc");
}

#[tokio::test]
async fn case_variant_chain_refuses_in_delta_walk() {
    let temp = TempRoot::new("casefold-delta");
    let git = git_executable();
    let root = repository(temp.path(), &git);
    let manifest = plan(
        &temp,
        &root,
        SnapshotSelection::CurrentWorkingContent,
        "casefold-delta",
    )
    .await
    .unwrap();
    let (artifacts, captured) = capture(&temp, &root, &manifest).await;
    let target = empty_private_target(temp.path(), "materialized");
    materialize_snapshot(
        &artifacts,
        manifest.snapshot_id(),
        &target,
        &CancellationToken::new(),
    )
    .unwrap();
    plant_case_variant_chain(&target);
    let error = compute_delta_at(
        &target,
        captured.snapshot_id(),
        captured.captured_manifest_sha256(),
        &captured,
        &CancellationToken::new(),
        DEFAULT_DELTA_DEADLINE,
    )
    .unwrap_err();
    assert!(
        matches!(error, DeltaError::UnsafePath),
        "the delta walk must refuse the folded chain: {error:?}"
    );
}
