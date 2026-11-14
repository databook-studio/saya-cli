//! Destination privacy and garbage tests: the artifacts root and its
//! staging directories stay 0700 and are refused when they are not, unsafe
//! snapshot ids never become artifact ids, stale staging sweeps away only
//! when its advisory lock is free while live stages and complete snapshots
//! stay untouched, and the cancel path leaves nothing.
//! Public API only, real temporary directories, small byte fixtures.

use std::{
    fs,
    io::Write,
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    path::Path,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use saya_harness::coding::snapshot::{SnapshotArtifacts, SnapshotError};
use saya_types::SnapshotId;

use super::super::support::TempRoot;
use super::fixtures::{MANIFEST_BYTES, artifacts_root, digest, write_object};

/// Root entries other than the coordination lock file `open` keeps: every
/// count assertion in this file is about the root's own content, not the
/// `.lock` file every destination carries.
pub(super) fn non_lock_entries(root: &Path) -> usize {
    fs::read_dir(root)
        .unwrap()
        .filter(|entry| entry.as_ref().unwrap().file_name() != ".lock")
        .count()
}

fn mode_of(path: &Path) -> u32 {
    fs::symlink_metadata(path).unwrap().permissions().mode()
}

pub(super) fn staging_entries(root: &Path) -> Vec<String> {
    let mut names = Vec::new();
    for entry in fs::read_dir(root).unwrap() {
        let name = entry.unwrap().file_name().to_string_lossy().into_owned();
        if name.starts_with(".staging-") {
            names.push(name);
        }
    }
    names
}

/// Nanoseconds one hour before now: a stranded fixture's recorded age is
/// parsed from its name, so the test embeds an ancient value there instead
/// of waiting on wall-clock behavior.
fn old_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        - 3_600_000_000_000
}

#[test]
fn staging_is_private_and_never_presents_as_complete() {
    let temp = TempRoot::new("w04-dest-private");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    assert_eq!(mode_of(&root) & 0o777, 0o700);

    let id = SnapshotId::parse("private-stage").unwrap();
    let mut staged = artifacts.begin(&id).unwrap();
    write_object(&mut staged, b"payload").unwrap();
    let staging = staging_entries(&root);
    assert_eq!(staging.len(), 1);
    assert!(staging[0].starts_with(".staging-private-stage-"));
    assert_eq!(mode_of(&root.join(&staging[0])) & 0o777, 0o700);
    assert!(!artifacts.is_complete(&id).unwrap());

    // The cancel path: an uncommitted stage leaves no final directory and
    // removes its own staging.
    drop(staged);
    assert!(staging_entries(&root).is_empty());
    assert_eq!(non_lock_entries(&root), 0);
    assert!(!artifacts.is_complete(&id).unwrap());
}

#[test]
fn snapshot_id_injection_refuses() {
    let temp = TempRoot::new("w04-dest-injection");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let over_long = "x".repeat(129);
    for injection in [
        "../x",
        "a/b",
        "",
        ".",
        "..",
        "a\0b",
        over_long.as_str(),
        "snåp",
    ] {
        // The typed boundary refuses every unsafe component shape, so no
        // injection string can ever reach the destination as an id.
        assert!(
            SnapshotId::parse(injection).is_err(),
            "{injection:?} refused"
        );
    }

    // A parseable id stages only inside the root, as one contained name.
    let id = SnapshotId::parse("injection-check").unwrap();
    let staged = artifacts.begin(&id).unwrap();
    let staging = staging_entries(&root);
    assert_eq!(staging.len(), 1);
    assert!(staging[0].starts_with(".staging-injection-check-"));
    drop(staged);
    assert_eq!(non_lock_entries(&root), 0);
}

#[test]
fn root_that_is_a_symlink_or_group_writable_is_refused() {
    let temp = TempRoot::new("w04-dest-root");
    let real = temp.path().join("real-root");
    fs::create_dir(&real).unwrap();
    fs::set_permissions(&real, fs::Permissions::from_mode(0o700)).unwrap();

    let linked = temp.path().join("linked-root");
    std::os::unix::fs::symlink(&real, &linked).unwrap();
    assert!(matches!(
        SnapshotArtifacts::open(&linked),
        Err(SnapshotError::ArtifactRootRefused)
    ));

    let group_writable = temp.path().join("group-root");
    fs::create_dir(&group_writable).unwrap();
    fs::set_permissions(&group_writable, fs::Permissions::from_mode(0o770)).unwrap();
    assert!(matches!(
        SnapshotArtifacts::open(&group_writable),
        Err(SnapshotError::ArtifactRootRefused)
    ));
    // The refusal never chmods a directory it does not own as private.
    assert_eq!(mode_of(&group_writable) & 0o777, 0o770);

    assert!(SnapshotArtifacts::open(&real).is_ok());
}

#[test]
fn sweep_keeps_live_and_locked_stages_and_removes_only_stale_unlocked() {
    let temp = TempRoot::new("w04-dest-sweep");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let published = SnapshotId::parse("sweep-keep").unwrap();
    let staged = artifacts.begin(&published).unwrap();
    staged
        .commit(MANIFEST_BYTES, &digest(MANIFEST_BYTES))
        .unwrap();

    // A live stage holds its advisory lock: even a zero bound skips it
    // without counting it, and its handle stays alive throughout.
    let live = SnapshotId::parse("sweep-live").unwrap();
    let mut live_stage = artifacts.begin(&live).unwrap();
    write_object(&mut live_stage, b"payload").unwrap();
    assert_eq!(artifacts.sweep_stale_staging(Duration::ZERO).unwrap(), 0);
    assert_eq!(staging_entries(&root).len(), 1);
    assert!(artifacts.is_complete(&published).unwrap());

    // Stages a killed process left behind are inert: their names carry
    // their own age, and their locks are free (the kernel released them at
    // process death) or absent (the process died before creating one).
    let old = old_nanos();
    let free_lock = root.join(format!(".staging-swept-free-1-{old}"));
    fs::create_dir(&free_lock).unwrap();
    fs::write(free_lock.join("lock"), b"").unwrap();
    let no_lock = root.join(format!(".staging-swept-none-1-{old}"));
    fs::create_dir(&no_lock).unwrap();
    // A stranded stage whose lock a live process still holds survives even
    // though its recorded age is ancient.
    let held = root.join(format!(".staging-swept-held-1-{old}"));
    fs::create_dir(&held).unwrap();
    fs::write(held.join("lock"), b"").unwrap();
    let held_fd = fs::OpenOptions::new()
        .read(true)
        .open(held.join("lock"))
        .unwrap();
    assert_eq!(
        unsafe { libc::flock(held_fd.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );

    assert_eq!(artifacts.sweep_stale_staging(Duration::ZERO).unwrap(), 2);
    assert!(!free_lock.exists());
    assert!(!no_lock.exists());
    assert!(held.exists());
    assert_eq!(staging_entries(&root).len(), 2);
    assert!(artifacts.is_complete(&published).unwrap());

    // Releasing the stranded lock makes it sweepable; the live stage never
    // does (its own drop removes its staging).
    drop(held_fd);
    // A child forked by another test thread can keep an inherited copy of the
    // dropped lock descriptor until it execs, so the release may land a
    // moment late: poll briefly for the sweep that finds it free.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let swept = loop {
        let swept = artifacts.sweep_stale_staging(Duration::ZERO).unwrap();
        if swept != 0 || std::time::Instant::now() >= deadline {
            break swept;
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    assert_eq!(swept, 1);
    assert!(!held.exists());
    drop(live_stage);
    assert!(staging_entries(&root).is_empty());
    assert_eq!(non_lock_entries(&root), 1);
    assert!(artifacts.is_complete(&published).unwrap());
}

#[test]
fn cancel_mid_capture_leaves_only_staging_garbage_that_is_swept() {
    let temp = TempRoot::new("w04-dest-cancel");
    let root = artifacts_root(&temp);
    let artifacts = SnapshotArtifacts::open(&root).unwrap();
    let id = SnapshotId::parse("cancelled-mid-write").unwrap();

    // Abandon an open object sink, then the stage itself: the cancel path.
    let mut staged = artifacts.begin(&id).unwrap();
    {
        let mut sink = staged.new_object().unwrap();
        sink.write_all(b"half-written object").unwrap();
    }
    drop(staged);
    assert!(staging_entries(&root).is_empty());
    assert_eq!(non_lock_entries(&root), 0);
    assert!(!artifacts.is_complete(&id).unwrap());

    // A stage a killed process could not clean is inert garbage the sweep
    // removes: its name carries an ancient age and its lock is free; it
    // never blocks the id and never presents as complete.
    let stranded = root.join(format!(".staging-{}-1-{}", id.as_str(), old_nanos()));
    fs::create_dir(&stranded).unwrap();
    fs::write(stranded.join("lock"), b"").unwrap();
    fs::write(stranded.join("object-00000000"), b"payload").unwrap();
    assert_eq!(artifacts.sweep_stale_staging(Duration::ZERO).unwrap(), 1);
    assert_eq!(non_lock_entries(&root), 0);
    assert!(!artifacts.is_complete(&id).unwrap());
}
