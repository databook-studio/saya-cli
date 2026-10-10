//! W04b materialization rollback (authored, not executed): a failure
//! after the target opens — a corrupted object mid-manifest, a corrupted
//! object past a ten-deep entry path, or case-fold colliding directory
//! paths in a COMPLETE-consistent rewritten manifest — refuses with a
//! typed error and rolls the target back to completely empty: no files,
//! no directories, zero `read_dir` entries. Public API only, real
//! temporary repositories.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::{SnapshotError, materialize_snapshot};
use saya_types::{Sha256Digest, SnapshotId};

use super::materialize::{
    artifacts_root, assert_target_empty, empty_private_target, publish_worktree,
};
use super::support::TempRoot;

/// An independent SHA-256 over the exact bytes, so no expected digest is
/// ever derived from the code under test.
fn digest(bytes: &[u8]) -> Sha256Digest {
    use sha2::{Digest as _, Sha256};
    use std::fmt::Write as _;
    let mut hex = String::new();
    for byte in Sha256::digest(bytes) {
        write!(&mut hex, "{byte:02x}").unwrap();
    }
    Sha256Digest::parse(&hex).unwrap()
}

/// Rewrites the published manifest.json with `edit` applied to its parsed
/// JSON body and refreshes COMPLETE to the new bytes' digest, so only the
/// materialization-side checks can refuse the run.
fn rewrite_manifest(dir: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
    let manifest_path = dir.join("manifest.json");
    let original = fs::read(&manifest_path).unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&original).unwrap();
    edit(&mut value);
    let bytes = serde_json::to_vec(&value).unwrap();
    fs::write(&manifest_path, &bytes).unwrap();
    fs::write(dir.join("COMPLETE"), digest(&bytes).as_str()).unwrap();
}

/// Flips one byte of the stored object `content` names, keeping the
/// length, so the object no longer hashes to its content-addressed name.
fn corrupt_object(temp: &TempRoot, id: &SnapshotId, content: &Sha256Digest) {
    let object = artifacts_root(temp)
        .join(id.as_str())
        .join("objects")
        .join(content.as_str());
    let mut bytes = fs::read(&object).unwrap();
    assert!(!bytes.is_empty());
    bytes[0] ^= 0xff;
    fs::write(&object, &bytes).unwrap();
}

#[tokio::test]
async fn rollback_after_partial_progress_removes_files_and_directories() {
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w04b-mat-rollback", |root| {
            fs::write(root.join("tracked.txt"), b"rollback probe bytes\n").unwrap();
            fs::create_dir_all(root.join("a/b")).unwrap();
            fs::write(root.join("a/one.txt"), b"nested one bytes\n").unwrap();
            fs::write(root.join("a/b/two.txt"), b"nested two bytes\n").unwrap();
            fs::write(root.join("c.txt"), b"root bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Premise: four entries, two of them nested, so the run creates
    // directories before the error fires.
    assert_eq!(captured.entries().len(), 4);
    assert!(
        captured
            .entries()
            .iter()
            .any(|entry| entry.path().as_str() == "a/one.txt")
    );
    assert!(
        captured
            .entries()
            .iter()
            .any(|entry| entry.path().as_str() == "a/b/two.txt")
    );
    // Corrupt the LAST manifest entry's stored object, keeping the
    // length: every earlier entry was already written when the copy
    // fails.
    let last = captured.entries().last().unwrap();
    corrupt_object(&temp, &id, last.content_sha256());
    let target = empty_private_target(temp.path(), "materialized");
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("a corrupted last object refuses materialization");
    assert!(matches!(refused, SnapshotError::ObjectDigestMismatch));
    // The rollback removed every created file and every created
    // directory: the target holds nothing at all.
    assert_target_empty(&target);
}

#[tokio::test]
async fn deep_paths_are_rolled_back_completely() {
    let deep = "d1/d2/d3/d4/d5/d6/d7/d8/d9/d10/deep.txt";
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w04b-mat-deep", |root| {
            fs::write(root.join("tracked.txt"), b"deep probe bytes\n").unwrap();
            fs::create_dir_all(root.join("d1/d2/d3/d4/d5/d6/d7/d8/d9/d10")).unwrap();
            fs::write(root.join(deep), b"deep bytes\n").unwrap();
            fs::write(root.join("z.txt"), b"last bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Premise: three entries, one ten directories deep, and the corrupted
    // entry is a later one in manifest order — the deep path was already
    // written when the copy fails.
    assert_eq!(captured.entries().len(), 3);
    assert!(
        captured
            .entries()
            .iter()
            .any(|entry| entry.path().as_str() == deep)
    );
    let last = captured.entries().last().unwrap();
    assert_ne!(last.path().as_str(), deep);
    corrupt_object(&temp, &id, last.content_sha256());
    let target = empty_private_target(temp.path(), "materialized");
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("a corrupted later object refuses materialization");
    assert!(matches!(refused, SnapshotError::ObjectDigestMismatch));
    // The rollback walked the whole depth: the target is empty, with no
    // leftover directory at any level.
    assert_target_empty(&target);
}

#[tokio::test]
async fn case_fold_colliding_directories_are_refused() {
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w04b-mat-casefold", |root| {
            fs::write(root.join("tracked.txt"), b"casefold probe bytes\n").unwrap();
            fs::write(root.join("x.txt"), b"upper probe bytes\n").unwrap();
            fs::write(root.join("y.txt"), b"lower probe bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Premise: at least two entries to retarget.
    assert!(captured.entries().len() >= 2);
    // No case-insensitive filesystem can hold two tracked files whose
    // paths differ only in directory case, so the collision is built by
    // rewriting the published manifest — COMPLETE refreshed to the new
    // bytes — with one entry under A/ and another under a/.
    let dir = artifacts_root(&temp).join(id.as_str());
    rewrite_manifest(&dir, |value| {
        let entries = value["entries"].as_array_mut().unwrap();
        assert!(entries.len() >= 2);
        entries[0]["path"] = serde_json::json!("A/x");
        entries[1]["path"] = serde_json::json!("a/y");
    });
    let target = empty_private_target(temp.path(), "materialized");
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("case-fold colliding directory paths refuse materialization");
    assert!(matches!(refused, SnapshotError::UnsafePath));
    // The refusal fired after the first entry was written, and the
    // rollback still left nothing behind.
    assert_target_empty(&target);
}
