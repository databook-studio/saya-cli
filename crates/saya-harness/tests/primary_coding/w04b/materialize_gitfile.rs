//! W05b materialization into a `--no-checkout` worktree (authored, not
//! executed): a target holding exactly one regular `.git` file — git's own
//! worktree pointer — materializes with the pointer left byte-identical and
//! untouched; every other shape (`.git` beside any entry, `.git` as a
//! directory or symlink) still refuses, and a failed run rolls back to the
//! `.git`-only target. Public API only, real temporary repositories.

use std::{fs, path::Path};

use saya_agent::CancellationToken;
use saya_harness::coding::snapshot::{SnapshotError, materialize_snapshot};
use saya_types::{Sha256Digest, SnapshotId};

use super::materialize::{artifacts_root, empty_private_target, publish_worktree};
use super::support::TempRoot;

/// The exact pointer-file bytes every test plants and re-checks, shaped like
/// what `git worktree add --no-checkout` writes.
const POINTER: &[u8] = b"gitdir: ../.git/worktrees/saya-checkout\n";

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

/// The sorted root entry names of the target, via public `read_dir`.
fn target_entries(target: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(target)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn dot_git_file_only_target_materializes() {
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w05b-dotgit-file", |root| {
            fs::write(root.join("tracked.txt"), b"baseline bytes\n").unwrap();
            fs::create_dir_all(root.join("a")).unwrap();
            fs::write(root.join("a/one.txt"), b"nested bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Premise: two approved entries, one nested, so the run creates a
    // directory beside the pointer file.
    assert_eq!(captured.entries().len(), 2);
    // `git worktree add --no-checkout` leaves exactly this: one regular
    // `.git` file pointing at the repository's admin area.
    let target = empty_private_target(temp.path(), "materialized");
    fs::write(target.join(".git"), POINTER).unwrap();
    materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new()).unwrap();
    // The pointer file survives with byte-identical content, beside the
    // materialized entries.
    assert_eq!(fs::read(target.join(".git")).unwrap(), POINTER);
    for entry in captured.entries() {
        assert!(target.join(entry.path().as_str()).exists());
    }
    assert_eq!(target_entries(&target).len(), captured.entries().len() + 1);
}

#[tokio::test]
async fn extra_entry_beside_dot_git_still_refuses() {
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w05b-dotgit-extra", |root| {
            fs::write(root.join("tracked.txt"), b"baseline bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    assert!(!captured.entries().is_empty());
    // `.git` beside any other entry is a populated directory, not a
    // no-checkout worktree: refuse before writing anything.
    let target = empty_private_target(temp.path(), "materialized");
    fs::write(target.join(".git"), POINTER).unwrap();
    fs::write(target.join("extra.txt"), b"extra\n").unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("an entry beside the `.git` pointer refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
    // Nothing changed: the target still holds exactly the pointer file and
    // the extra entry, with the pointer bytes untouched.
    assert_eq!(target_entries(&target), vec![".git", "extra.txt"]);
    assert_eq!(fs::read(target.join(".git")).unwrap(), POINTER);
}

#[tokio::test]
async fn dot_git_symlink_or_directory_refuses() {
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w05b-dotgit-shape", |root| {
            fs::write(root.join("tracked.txt"), b"baseline bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    assert!(!captured.entries().is_empty());
    // A symlinked `.git` is not the worktree pointer: refuse.
    let symlinked = empty_private_target(temp.path(), "materialized-symlink");
    std::os::unix::fs::symlink(temp.path().join("elsewhere"), symlinked.join(".git")).unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &symlinked, &CancellationToken::new())
        .expect_err("a symlinked `.git` refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
    assert_eq!(target_entries(&symlinked), vec![".git"]);
    // A directory `.git` is not the worktree pointer either: refuse.
    let directory = empty_private_target(temp.path(), "materialized-dir");
    fs::create_dir(directory.join(".git")).unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &directory, &CancellationToken::new())
        .expect_err("a directory `.git` refuses materialization");
    assert!(matches!(refused, SnapshotError::MaterializeTargetInvalid));
    assert_eq!(target_entries(&directory), vec![".git"]);
}

#[tokio::test]
async fn rollback_preserves_dot_git() {
    let (temp, _root, artifacts, captured, _manifest, id) =
        publish_worktree("w05b-dotgit-rollback", |root| {
            fs::write(root.join("tracked.txt"), b"rollback probe bytes\n").unwrap();
            fs::create_dir_all(root.join("a/b")).unwrap();
            fs::write(root.join("a/one.txt"), b"nested one bytes\n").unwrap();
            fs::write(root.join("a/b/two.txt"), b"nested two bytes\n").unwrap();
        })
        .await;
    assert!(artifacts.is_complete(&id).unwrap());
    // Premise: three entries, two nested, and the corrupted entry is the
    // last one in manifest order — files were already written when the
    // copy fails.
    assert_eq!(captured.entries().len(), 3);
    let last = captured.entries().last().unwrap();
    corrupt_object(&temp, &id, last.content_sha256());
    let target = empty_private_target(temp.path(), "materialized");
    fs::write(target.join(".git"), POINTER).unwrap();
    let refused = materialize_snapshot(&artifacts, &id, &target, &CancellationToken::new())
        .expect_err("a corrupted last object refuses materialization");
    assert!(matches!(refused, SnapshotError::ObjectDigestMismatch));
    // The rollback removed every entry the run created and left the target
    // holding only the unchanged `.git` pointer file.
    assert_eq!(target_entries(&target), vec![".git"]);
    assert_eq!(fs::read(target.join(".git")).unwrap(), POINTER);
}
