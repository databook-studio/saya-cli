//! Publish and verification tests: the atomic no-replace commit with its
//! verifiable completion record, digest-mismatch and tamper refusals, the
//! manifest bound, object content identity, and completion verification
//! that cannot be hung by a FIFO. Public API only, real temporary
//! directories, small byte fixtures.

use std::{ffi::CString, fs, io::Write, os::unix::ffi::OsStrExt, path::Path};

use saya_harness::coding::snapshot::{
    MAX_SNAPSHOT_MANIFEST_BYTES, SnapshotArtifacts, SnapshotError,
};
use saya_types::SnapshotId;

use super::super::support::TempRoot;
use super::destination::{non_lock_entries, staging_entries};
use super::fixtures::{MANIFEST_BYTES, artifacts_root, digest, write_object};

#[test]
fn commit_publishes_atomically_and_is_complete() {
    let temp = TempRoot::new("w04-dest-commit");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("commit-me").unwrap();
    let manifest_digest = digest(MANIFEST_BYTES);

    let mut staged = artifacts.begin(&id).unwrap();
    write_object(&mut staged, b"object-bytes").unwrap();
    // Identical verified content promotes under the same name: the store
    // dedups, and the right digest stores the object under its own name.
    write_object(&mut staged, b"object-bytes").unwrap();
    staged.commit(MANIFEST_BYTES, &manifest_digest).unwrap();

    let final_dir = root.join(id.as_str());
    assert!(fs::symlink_metadata(&final_dir).unwrap().is_dir());
    assert!(staging_entries(&root).is_empty());
    assert_eq!(
        fs::read(final_dir.join("manifest.json")).unwrap(),
        MANIFEST_BYTES
    );
    // The completion record is written last and carries the manifest digest.
    assert_eq!(
        fs::read(final_dir.join("COMPLETE")).unwrap(),
        manifest_digest.as_str().as_bytes()
    );
    // The stage's advisory lock file never publishes.
    assert_eq!(fs::read_dir(&final_dir).unwrap().count(), 3);
    let object_digest = digest(b"object-bytes");
    let object = final_dir.join("objects").join(object_digest.as_str());
    assert_eq!(fs::read(&object).unwrap(), b"object-bytes");
    assert_eq!(fs::read_dir(final_dir.join("objects")).unwrap().count(), 1);
    assert!(artifacts.is_complete(&id).unwrap());

    // A published id is never staged again.
    assert!(matches!(
        artifacts.begin(&id),
        Err(SnapshotError::SnapshotExists)
    ));
}

#[test]
fn wrong_object_digest_refuses_and_stores_nothing() {
    let temp = TempRoot::new("w04-dest-object-digest");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("digest-check").unwrap();

    let mut staged = artifacts.begin(&id).unwrap();
    let mut sink = staged.new_object().unwrap();
    sink.write_all(b"payload").unwrap();
    assert!(matches!(
        sink.finish(&digest(b"different bytes")),
        Err(SnapshotError::ObjectDigestMismatch)
    ));
    // The temp is unlinked: nothing was stored under the supplied name.
    let staging = staging_entries(&root);
    assert_eq!(staging.len(), 1);
    assert_eq!(
        fs::read_dir(root.join(&staging[0]).join("objects"))
            .unwrap()
            .count(),
        0
    );
    drop(staged);
    assert_eq!(non_lock_entries(&root), 0);
}

#[test]
fn commit_with_wrong_manifest_digest_refuses_and_leaves_no_final_dir() {
    let temp = TempRoot::new("w04-dest-wrong-digest");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("wrong-digest").unwrap();

    let mut staged = artifacts.begin(&id).unwrap();
    write_object(&mut staged, b"object-bytes").unwrap();
    assert!(matches!(
        staged.commit(MANIFEST_BYTES, &digest(b"different bytes")),
        Err(SnapshotError::ManifestDigestMismatch)
    ));
    // The stage is gone and nothing was ever published under the id; only
    // the persistent `.lock` entry `open` keeps remains.
    assert_eq!(non_lock_entries(&root), 0);
    assert!(!artifacts.is_complete(&id).unwrap());
}

#[test]
fn manifest_over_bound_refuses_without_publishing() {
    let temp = TempRoot::new("w04-dest-manifest-bound");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("manifest-bound").unwrap();

    let staged = artifacts.begin(&id).unwrap();
    // One byte over the manifest bound is refused whole, before any file
    // is written; the 8 MiB+1 transient stays a small fixture.
    let oversized = vec![b'a'; MAX_SNAPSHOT_MANIFEST_BYTES + 1];
    assert!(matches!(
        staged.commit(&oversized, &digest(&oversized)),
        Err(SnapshotError::TooLarge)
    ));
    // `commit` consumed the stage, so there is no handle left to drop; the
    // refused commit cleans its staging on the consumed value's own path.
    assert_eq!(non_lock_entries(&root), 0);
}

#[test]
fn tampered_completion_record_or_manifest_is_not_complete() {
    let temp = TempRoot::new("w04-dest-tamper");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("tampered").unwrap();
    let manifest_digest = digest(MANIFEST_BYTES);
    let final_dir = root.join(id.as_str());

    let staged = artifacts.begin(&id).unwrap();
    staged.commit(MANIFEST_BYTES, &manifest_digest).unwrap();
    assert!(artifacts.is_complete(&id).unwrap());

    // A rewritten record with another valid digest does not verify.
    fs::write(final_dir.join("COMPLETE"), digest(b"other").as_str()).unwrap();
    assert!(!artifacts.is_complete(&id).unwrap());

    // A restored record does not rescue a tampered manifest.
    fs::write(final_dir.join("COMPLETE"), manifest_digest.as_str()).unwrap();
    fs::write(final_dir.join("manifest.json"), b"{\"tampered\":true}").unwrap();
    assert!(!artifacts.is_complete(&id).unwrap());

    // A missing record is never an apparent-complete snapshot.
    fs::write(final_dir.join("manifest.json"), MANIFEST_BYTES).unwrap();
    fs::remove_file(final_dir.join("COMPLETE")).unwrap();
    assert!(!artifacts.is_complete(&id).unwrap());
}

/// A FIFO named like a completion file must read as not complete, and the
/// nonblocking open must return synchronously: a blocking call would hang
/// this test outright.
#[test]
fn fifo_at_completion_names_is_not_complete_and_does_not_block() {
    let temp = TempRoot::new("w04-dest-fifo");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("fifo-guarded").unwrap();
    let final_dir = root.join(id.as_str());

    let staged = artifacts.begin(&id).unwrap();
    staged
        .commit(MANIFEST_BYTES, &digest(MANIFEST_BYTES))
        .unwrap();
    assert!(artifacts.is_complete(&id).unwrap());

    fs::remove_file(final_dir.join("COMPLETE")).unwrap();
    make_fifo(&final_dir.join("COMPLETE"));
    assert!(!artifacts.is_complete(&id).unwrap());

    fs::remove_file(final_dir.join("manifest.json")).unwrap();
    make_fifo(&final_dir.join("manifest.json"));
    assert!(!artifacts.is_complete(&id).unwrap());
}

fn make_fifo(path: &Path) {
    let c = CString::new(path.as_os_str().as_bytes()).unwrap();
    assert_eq!(
        unsafe { libc::mkfifo(c.as_ptr(), 0o600 as libc::mode_t) },
        0
    );
}
