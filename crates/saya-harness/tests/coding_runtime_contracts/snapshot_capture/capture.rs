//! Capture-orchestration tests (authored, not executed): the public W04d
//! producer composes plan → per-entry capture → published artifacts without
//! touching the sources, the captured manifest is a distinct self-verifying
//! digest, a Head entry ignores a dirty working overlay, and `Debug` never
//! prints a path. Public API only, real temporary repositories.

use std::{collections::BTreeMap, fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::{
    git::{GitRepository, GitService},
    snapshot::{
        CapturedSnapshot, SnapshotArtifacts, SnapshotError, SnapshotManifest,
        SnapshotOmissionPolicy, SnapshotSelection,
    },
};
use saya_harness::workspace::Workspace;
use saya_types::SnapshotId;

use super::super::support::{TempRoot, git_args, git_executable, repository, run_git, state_root};
use super::fixtures::{artifacts_root, digest};

/// Builds one real Git repository, lets `prepare` shape the working tree,
/// commits `tracked.txt` for a Head selection, then plans through the
/// public W03 producer.
async fn plan_selection(
    label: &str,
    snapshot_id: &str,
    prepare: impl FnOnce(&Path),
    selection: SnapshotSelection,
) -> (TempRoot, GitService, GitRepository, SnapshotManifest) {
    let temp = TempRoot::new(label);
    let git = git_executable();
    let root = repository(temp.path(), &git);
    prepare(&root);
    if selection == SnapshotSelection::Head {
        run_git(
            &git,
            temp.path(),
            git_args(&root, &["add", "--", "tracked.txt"]),
        );
        run_git(
            &git,
            temp.path(),
            git_args(&root, &["commit", "--quiet", "-m", "tracked"]),
        );
    }
    let service = GitService::new(&git, state_root(temp.path())).unwrap();
    let cancellation = CancellationToken::new();
    let opened = service.open_repository(&root, &cancellation).await.unwrap();
    let policy = SnapshotOmissionPolicy::new(&opened, Vec::new()).unwrap();
    let manifest = service
        .plan_snapshot(
            &opened,
            SnapshotId::parse(snapshot_id).unwrap(),
            selection,
            policy,
            &cancellation,
        )
        .await
        .unwrap();
    (temp, service, opened, manifest)
}

/// Captures through the single public W04d producer with a fresh token.
async fn capture(
    service: &GitService,
    repository: &GitRepository,
    workspace: &Workspace,
    manifest: &SnapshotManifest,
    artifacts: &SnapshotArtifacts,
) -> Result<CapturedSnapshot, SnapshotError> {
    service
        .capture_snapshot(
            repository,
            workspace,
            manifest,
            artifacts,
            &CancellationToken::new(),
        )
        .await
}

#[tokio::test]
async fn capture_matches_plan_without_touching_source() {
    let (temp, service, opened, manifest) = plan_selection(
        "w04-capture-match",
        "w04-capture-match",
        |root| {
            fs::write(root.join("tracked.txt"), b"first tracked bytes\n").unwrap();
            fs::write(root.join("extra.bin"), b"extra payload bytes\n").unwrap();
        },
        SnapshotSelection::CurrentWorkingContent,
    )
    .await;
    let root = opened.root().to_path_buf();
    // Every planned entry's exact bytes, read before capture: comparison is
    // by content, never by mtime.
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
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = capture(&service, &opened, &workspace, &manifest, &artifacts)
        .await
        .unwrap();
    assert!(artifacts.is_complete(&id).unwrap());
    let captured_entries = captured.entries();
    assert_eq!(captured_entries.len(), manifest.entries().len());
    let objects = artifacts_root(&temp).join(id.as_str()).join("objects");
    for (plan_entry, captured_entry) in manifest.entries().iter().zip(captured_entries) {
        // The result lists every planned entry, in plan order.
        assert_eq!(plan_entry.path().as_str(), captured_entry.path().as_str());
        assert_eq!(plan_entry.mode(), captured_entry.mode());
        // The source file's bytes are exactly what they were before.
        let path = root.join(plan_entry.path().as_str());
        assert_eq!(
            fs::read(&path).unwrap().as_slice(),
            before[plan_entry.path().as_str()].as_slice()
        );
        // The artifact object is content-addressed by the entry's digest
        // and holds those same bytes, independently re-hashed.
        let hex = captured_entry.content_sha256().as_str();
        let stored = fs::read(objects.join(hex)).unwrap();
        assert_eq!(
            stored.as_slice(),
            before[plan_entry.path().as_str()].as_slice()
        );
        assert_eq!(digest(&stored).as_str(), hex);
        assert_eq!(captured_entry.captured_bytes(), stored.len() as u64);
    }
    assert_eq!(
        fs::read_dir(objects).unwrap().count(),
        captured_entries.len()
    );
}

#[tokio::test]
async fn captured_manifest_digest_distinct_from_plan_digest_and_self_verifying() {
    let (temp, service, opened, manifest) = plan_selection(
        "w04-capture-digest",
        "w04-capture-digest",
        |root| fs::write(root.join("tracked.txt"), b"digest probe bytes\n").unwrap(),
        SnapshotSelection::CurrentWorkingContent,
    )
    .await;
    let id = manifest.snapshot_id().clone();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = capture(&service, &opened, &workspace, &manifest, &artifacts)
        .await
        .unwrap();
    // The captured digest is a different field and a different value from
    // the plan digest.
    assert_ne!(
        captured.captured_manifest_sha256().as_str(),
        manifest.manifest_sha256().as_str()
    );
    // Self-verifying: the digest names the exact stored manifest.json
    // bytes, whose serialized body opens with the fixed field order.
    let stored = fs::read(
        artifacts_root(&temp)
            .join(id.as_str())
            .join("manifest.json"),
    )
    .unwrap();
    assert_eq!(
        digest(&stored).as_str(),
        captured.captured_manifest_sha256().as_str()
    );
    assert!(stored.starts_with(b"{\"schema_version\":1"));
}

#[tokio::test]
async fn head_mode_ignores_dirty_overlay_in_capture() {
    let contents: &[u8] = b"committed head bytes\n";
    let (temp, service, opened, manifest) = plan_selection(
        "w04-capture-head",
        "w04-capture-head",
        |root| fs::write(root.join("tracked.txt"), contents).unwrap(),
        SnapshotSelection::Head,
    )
    .await;
    // A dirty working-tree overlay left after planning: Head capture must
    // take the committed blob bytes and ignore the overlay entirely.
    fs::write(opened.root().join("tracked.txt"), b"dirty overlay bytes\n").unwrap();
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = capture(&service, &opened, &workspace, &manifest, &artifacts)
        .await
        .unwrap();
    let entry = captured
        .entries()
        .iter()
        .find(|entry| entry.path().as_str() == "tracked.txt")
        .expect("the committed file is captured");
    let stored = fs::read(
        artifacts_root(&temp)
            .join(manifest.snapshot_id().as_str())
            .join("objects")
            .join(entry.content_sha256().as_str()),
    )
    .unwrap();
    assert_eq!(stored, contents);
    assert_eq!(digest(&stored).as_str(), entry.content_sha256().as_str());
}

#[tokio::test]
async fn debug_of_the_result_prints_no_path() {
    let (temp, service, opened, manifest) = plan_selection(
        "w04-capture-debug-root",
        "w04-capdbg",
        |root| fs::write(root.join("tracked.txt"), b"debug probe bytes\n").unwrap(),
        SnapshotSelection::CurrentWorkingContent,
    )
    .await;
    let artifacts = SnapshotArtifacts::open(&artifacts_root(&temp)).unwrap();
    let workspace = Workspace::open(opened.root()).unwrap();
    let captured = capture(&service, &opened, &workspace, &manifest, &artifacts)
        .await
        .unwrap();
    let rendered = format!("{captured:?}");
    // Only ids, digests, and counts print: never the repository path, the
    // artifacts root, or a snapshot path.
    assert!(!rendered.contains("tracked"));
    assert!(!rendered.contains(temp.path().to_str().unwrap()));
    assert!(!rendered.contains(opened.root().to_str().unwrap()));
    assert!(!rendered.contains(artifacts_root(&temp).to_str().unwrap()));
}
