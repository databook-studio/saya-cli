//! W04b materialization refusals (authored, not executed): a non-empty
//! target, a 0755 target, a plain-file target, and a symlinked target
//! path over an empty private directory all refuse with
//! `MaterializeTargetInvalid`; a corrupted artifact object, a symlinked
//! artifact object, a cancelled run, and an unknown snapshot id refuse
//! with their typed errors — and partial runs never leave anything
//! behind. Public API only, real temporary repositories.

use std::{fs, os::unix::fs::PermissionsExt};

use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::{SnapshotError, materialize_snapshot};
use saya_types::SnapshotId;

use super::materialize::{
    artifacts_root, assert_target_empty, empty_private_target, publish_worktree,
};

#[tokio::test]
async fn target_must_be_empty_private_dir() {
    let (temp, _root, artifacts, _captured, _manifest, id) =
        publish_worktree("w04b-mat-target", |root| {
            fs::write(root.join("tracked.txt"), b"target probe bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // A non-empty target refuses: the directory holds one stale file.
    let occupied = empty_private_target(temp.path(), "occupied");
    fs::write(occupied.join("stale.txt"), b"stale bytes\n").unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &occupied, &CancellationToken::new())
        .expect_err("a non-empty target refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
    // A 0755 target refuses: privacy is materialization's own check.
    let loose = empty_private_target(temp.path(), "loose");
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o755)).unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &loose, &CancellationToken::new())
        .expect_err("a 0755 target refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
    // A plain file cannot be a target.
    let file_target = temp.path().join("target-is-a-file");
    fs::write(&file_target, b"not a directory\n").unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &file_target, &CancellationToken::new())
        .expect_err("a file target refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
    // A symlink at the target path refuses even when it points at an
    // empty private directory: the boundary refuses the link itself, so
    // only the symlink refusal can fire.
    let linked = temp.path().join("target-is-a-symlink");
    let behind = empty_private_target(temp.path(), "behind");
    std::os::unix::fs::symlink(&behind, &linked).unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &linked, &CancellationToken::new())
        .expect_err("a symlinked target path refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
}

#[tokio::test]
async fn corrupted_object_refuses_and_leaves_empty_target() {
    let (temp, _root, artifacts, _captured, _manifest, id) =
        publish_worktree("w04b-mat-object", |root| {
            fs::write(root.join("tracked.txt"), b"corruption probe bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Flip one byte of one artifact object in place, keeping the length:
    // the object no longer hashes to its content-addressed name.
    let objects = artifacts_root(&temp).join(id.as_str()).join("objects");
    let object = fs::read_dir(&objects)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = fs::read(&object).unwrap();
    assert!(!bytes.is_empty());
    bytes[0] ^= 0xff;
    fs::write(&object, &bytes).unwrap();
    let target = empty_private_target(temp.path(), "materialized");
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("a corrupted artifact object refuses materialization");
    assert!(matches!(refused, SnapshotError::ObjectDigestMismatch));
    // The partial run rolled back: the target is empty again.
    assert_target_empty(&target);
}

#[tokio::test]
async fn symlink_in_target_refuses() {
    // A symlink cannot be planted inside the verified-empty target, so the
    // no-follow rule is exercised on the object store instead: one artifact
    // object is replaced by a symlink and materialization must refuse
    // without following it.
    let (temp, _root, artifacts, _captured, _manifest, id) =
        publish_worktree("w04b-mat-symlink", |root| {
            fs::write(root.join("tracked.txt"), b"symlink probe bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    let objects = artifacts_root(&temp).join(id.as_str()).join("objects");
    let object = fs::read_dir(&objects)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_file(&object).unwrap();
    std::os::unix::fs::symlink(
        artifacts_root(&temp)
            .join(id.as_str())
            .join("manifest.json"),
        &object,
    )
    .unwrap();
    let target = empty_private_target(temp.path(), "materialized");
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("a symlinked artifact object refuses materialization");
    // The no-follow open refuses the symlink itself: an artifact I/O
    // refusal, not a digest mismatch.
    assert!(matches!(refused, SnapshotError::CaptureIo { .. }));
    assert_target_empty(&target);
}

#[tokio::test]
async fn cancelled_materialization_leaves_empty_target() {
    let (temp, _root, artifacts, _captured, _manifest, id) =
        publish_worktree("w04b-mat-cancel", |root| {
            fs::write(root.join("tracked.txt"), b"cancel probe bytes\n").unwrap();
            fs::write(root.join("extra.txt"), b"more cancel probe bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    let target = empty_private_target(temp.path(), "materialized");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let refused = materialize_snapshot(&artifacts, &id, &target, &cancellation)
        .expect_err("a cancelled materialization refuses");
    assert!(matches!(refused, SnapshotError::Cancelled));
    // Nothing stays behind: the target is empty again.
    assert_target_empty(&target);
}

#[tokio::test]
async fn unknown_snapshot_refuses() {
    let (temp, _root, artifacts, _captured, _manifest, _id) =
        publish_worktree("w04b-mat-unknown", |root| {
            fs::write(root.join("tracked.txt"), b"unknown probe bytes\n").unwrap();
        })
        .await;
    let ghost = SnapshotId::parse("w04b-mat-ghost").unwrap();
    let target = empty_private_target(temp.path(), "materialized");
    let refused = materialize_snapshot(&artifacts, &ghost, &target, &CancellationToken::new())
        .expect_err("an unpublished snapshot id refuses materialization");
    assert!(matches!(refused, SnapshotError::SnapshotNotFound));
}
