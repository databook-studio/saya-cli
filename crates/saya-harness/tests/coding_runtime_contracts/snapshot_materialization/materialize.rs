//! W04b materialization acceptance (authored, not executed): a real
//! worktree snapshot materializes into a fresh empty private target that
//! holds exactly the approved entries — same bytes, exact 0644/0755 file
//! modes, 0700 directories, nothing extra — with captured, receipt, and
//! walked path sets equal by membership and count. Public API only.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::GitService,
    snapshot::{
        CapturedSnapshot, SnapshotArtifacts, SnapshotManifest, SnapshotOmissionPolicy,
        SnapshotSelection, materialize_snapshot,
    },
};
use saya_harness::workspace::Workspace;
use saya_types::SnapshotId;

use super::support::{TempRoot, git_executable, repository, state_root};

/// The fixture artifacts root: `<artifacts>/<id>/objects/<sha256>` per the capture layout.
pub(super) fn artifacts_root(temp: &TempRoot) -> PathBuf {
    temp.path().join("artifacts")
}

/// Creates an empty, 0700 directory under `parent` as the materialization target.
pub(super) fn empty_private_target(parent: &Path, name: &str) -> PathBuf {
    let target = parent.join(name);
    fs::create_dir(&target).unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).unwrap();
    target
}

/// Asserts the target holds nothing: no files, no directories, zero `read_dir` entries.
pub(super) fn assert_target_empty(target: &Path) {
    assert_eq!(fs::read_dir(target).unwrap().count(), 0);
}

/// Every file and directory under `root` as relative slash paths mapped to permission modes.
struct WalkedTree {
    files: BTreeMap<String, u32>,
    dirs: BTreeMap<String, u32>,
}

fn walked_tree(root: &Path) -> WalkedTree {
    fn walk(dir: &Path, prefix: &str, tree: &mut WalkedTree) {
        for entry in fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel = format!("{prefix}{name}");
            let metadata = fs::symlink_metadata(entry.path()).unwrap();
            let mode = metadata.permissions().mode() & 0o7777;
            if metadata.is_dir() {
                tree.dirs.insert(rel.clone(), mode);
                walk(&entry.path(), &format!("{rel}/"), tree);
            } else {
                tree.files.insert(rel, mode);
            }
        }
    }
    let mut tree = WalkedTree {
        files: BTreeMap::new(),
        dirs: BTreeMap::new(),
    };
    walk(root, "", &mut tree);
    tree
}

/// Every ancestor directory of `path` ("a/b/two.txt" yields "a" and "a/b"); roots yield none.
fn parent_directories(path: &str) -> Vec<String> {
    let bytes = path.as_bytes();
    (1..bytes.len())
        .filter(|index| bytes[*index] == b'/')
        .map(|index| path[..index].to_owned())
        .collect()
}

/// Builds one real Git repository, lets `prepare` shape the working tree on
/// top of the committed `tracked.txt`, then plans the current working
/// content and captures it through the public W04d producer.
pub(super) async fn publish_worktree(
    label: &str,
    prepare: impl FnOnce(&Path),
) -> (
    TempRoot,
    PathBuf,
    SnapshotArtifacts,
    CapturedSnapshot,
    SnapshotManifest,
    SnapshotId,
) {
    let temp = TempRoot::new(label);
    let git = git_executable();
    let root = repository(temp.path(), &git);
    prepare(&root);
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(label).unwrap(),
            SnapshotSelection::CurrentWorkingContent,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = service
        .capture_snapshot(&opened, &workspace, &manifest, &artifacts, &cancellation)
        .await
        .unwrap();
    (temp, root, artifacts, captured, manifest, id)
}

#[tokio::test]
async fn snapshot_materialization_matches_manifest() {
    let (temp, root, artifacts, captured, manifest, id) =
        publish_worktree("w04b-mat-match", |root| {
            fs::write(root.join("tracked.txt"), b"modified tracked bytes\n").unwrap();
            fs::write(root.join("untracked.txt"), b"untracked bytes\n").unwrap();
            fs::create_dir_all(root.join("a/b")).unwrap();
            fs::write(root.join("a/one.txt"), b"nested one bytes\n").unwrap();
            fs::write(root.join("a/b/two.txt"), b"nested two bytes\n").unwrap();
            fs::write(root.join("exec.sh"), b"#!/bin/sh\nexec true\n").unwrap();
            fs::set_permissions(root.join("exec.sh"), fs::Permissions::from_mode(0o755)).unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Premise: the fixture captures exactly the modified tracked file, the
    // untracked file, the two nested files, and the executable file.
    assert_eq!(captured.entries().len(), 5);
    assert!(
        captured
            .entries()
            .iter()
            .any(|entry| entry.path().as_str() == "exec.sh")
    );
    // The source files' exact bytes as capture approved them (capture never
    // touches the sources).
    let before: BTreeMap<String, Vec<u8>> = manifest
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.path().as_str().to_owned(),
                fs::read(root.join(entry.path().as_str())).unwrap(),
            )
        })
        .collect();
    let target = empty_private_target(temp.path(), "materialized");
    let materialized =
        materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new()).unwrap();
    // The receipt echoes the approved snapshot id and both digests.
    assert_eq!(materialized.snapshot_id().as_str(), id.as_str());
    assert_eq!(
        materialized.plan_manifest_sha256().as_str(),
        manifest.manifest_sha256().as_str()
    );
    assert_eq!(
        materialized.captured_manifest_sha256().as_str(),
        captured.captured_manifest_sha256().as_str()
    );
    // The receipt entries equal the capture result as complete maps —
    // every path with its mode, content digest, and byte count — not a
    // truncating zip.
    let receipt: BTreeMap<_, _> = materialized
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.path().as_str().to_owned(),
                (
                    entry.mode(),
                    entry.content_sha256().as_str().to_owned(),
                    entry.bytes(),
                ),
            )
        })
        .collect();
    let approved: BTreeMap<_, _> = captured
        .entries()
        .iter()
        .map(|entry| {
            (
                entry.path().as_str().to_owned(),
                (
                    entry.mode(),
                    entry.content_sha256().as_str().to_owned(),
                    entry.captured_bytes(),
                ),
            )
        })
        .collect();
    assert_eq!(receipt, approved);
    // The captured paths, the receipt paths, and the walked target files
    // are one set with one count: nothing omitted, nothing extra.
    let captured_paths: BTreeSet<String> = captured
        .entries()
        .iter()
        .map(|entry| entry.path().as_str().to_owned())
        .collect();
    assert_eq!(captured_paths.len(), captured.entries().len());
    let receipt_paths: BTreeSet<String> = receipt.keys().cloned().collect();
    let tree = walked_tree(&target);
    let walked_paths: BTreeSet<String> = tree.files.keys().cloned().collect();
    assert_eq!(captured_paths, receipt_paths);
    assert_eq!(receipt_paths, walked_paths);
    assert_eq!(walked_paths.len(), materialized.entries().len());
    // The target holds exactly the parents of the approved entries as
    // directories — and nothing else.
    let expected_dirs: BTreeSet<String> = captured_paths
        .iter()
        .flat_map(|path| parent_directories(path))
        .collect();
    assert!(expected_dirs.contains("a") && expected_dirs.contains("a/b"));
    assert_eq!(
        tree.dirs.keys().cloned().collect::<BTreeSet<_>>(),
        expected_dirs
    );
    // Every created directory is private at 0700.
    for mode in tree.dirs.values() {
        assert_eq!(*mode, 0o700);
    }
    // Every file lands with the source bytes and an exact mode: 0755 for
    // the fixture's executable, 0644 for every regular file.
    for (path, mode) in &tree.files {
        assert_eq!(
            fs::read(target.join(path)).unwrap().as_slice(),
            before[path].as_slice()
        );
        if path.as_str() == "exec.sh" {
            assert_eq!(*mode, 0o755);
        } else {
            assert_eq!(*mode, 0o644);
        }
    }
}
