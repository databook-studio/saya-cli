//! The tombstone half of a job's removal: the records are renamed out of the
//! job's name before anything is taken, a stop leaves one `.deleting-*` entry
//! and never a half-deleted job, and the next removal finishes it.

use super::tests::{job, names, records, scratch};
use super::tombstone::{Kind, kind};
use super::*;

/// A job whose own directory is past the per-directory bound: one removal
/// cannot take it in one pass.
fn over_bound(root: &Path, id: &str) -> PathBuf {
    let directory = records(root, id);
    for index in 0..4097 {
        fs::write(directory.join(format!("record-{index}")), "x").expect("a record is writable");
    }
    directory
}

#[test]
fn a_bound_trip_leaves_a_tombstone_and_never_a_half_deleted_job() {
    let root = scratch("bound");
    let directory = over_bound(&root, "job-bound");
    let removed = delete_records(&root, &job("job-bound"));
    assert!(
        removed.is_err(),
        "a tree past the per-directory bound is refused: {removed:?}"
    );
    assert!(
        !root.join("job-bound").exists(),
        "the records were renamed out of the job's name before anything was removed"
    );
    let listed = names(&root);
    assert_eq!(
        listed.len(),
        1,
        "one tombstone is all a stopped delete leaves: {listed:?}"
    );
    assert!(
        listed[0].starts_with(".deleting-"),
        "the leftover is the tombstone: {listed:?}"
    );
    let _ = fs::remove_dir_all(root);
    let _ = directory;
}

#[test]
fn the_next_delete_finishes_the_tombstone_a_stopped_delete_left() {
    let root = scratch("finish");
    over_bound(&root, "job-bound");
    assert!(
        delete_records(&root, &job("job-bound")).is_err(),
        "the first removal stops at the bound"
    );
    let finished = delete_records(&root, &job("job-bound"));
    assert!(
        finished.is_ok(),
        "the next delete finishes it: {finished:?}"
    );
    assert!(
        names(&root).is_empty(),
        "nothing of the job is left, the tombstone included: {:?}",
        names(&root)
    );
    let _ = fs::remove_dir_all(root);
}

/// A pass over a directory past the per-directory bound takes the names it
/// read and reports the bound; what it took stays taken, so the next pass — or
/// the next delete — converges instead of meeting the same bound forever.
#[test]
fn a_bounded_pass_makes_progress_so_the_next_pass_converges() {
    let root = scratch("progress");
    let directory = records(&root, "job-progress");
    let total = MAX_ENTRIES + 1;
    for index in 0..total {
        fs::write(directory.join(format!("record-{index}")), "x").expect("a record is writable");
    }
    let before = fs::read_dir(&directory)
        .map(|entries| entries.count())
        .unwrap_or_default();
    let bounded = remove_tree(&directory, MAX_DEPTH);
    assert!(
        bounded.is_err(),
        "a directory past the per-directory bound is refused: {bounded:?}"
    );
    let left = fs::read_dir(&directory)
        .map(|entries| entries.count())
        .unwrap_or_default();
    assert_eq!(
        before - left,
        MAX_ENTRIES,
        "the pass removed the names it read: {left} of {before} entries left"
    );
    remove_tree(&directory, MAX_DEPTH).expect("the next pass finishes it");
    assert!(
        !directory.exists(),
        "the bound converges: the next pass leaves nothing"
    );
    let _ = fs::remove_dir_all(root);
}

/// The outside folder a planted link points at: one file a removal that
/// followed the link would take.
#[cfg(unix)]
fn outside(root: &Path) -> PathBuf {
    let outside = root.join("outside");
    fs::create_dir_all(&outside).expect("the folder outside is creatable");
    fs::write(outside.join("keep"), "kept").expect("a file outside is writable");
    outside
}

/// A name a subdirectory would hold is a link to a folder outside: the swap
/// stands in for the window between classifying an entry and entering it.
#[cfg(unix)]
#[test]
fn a_directory_swapped_for_a_link_is_unlinked_and_never_entered() {
    let root = scratch("swapped");
    let outside = outside(&root);
    let directory = records(&root, "job-swapped");
    fs::remove_dir_all(directory.join("checkpoints")).expect("the subdirectory is removable");
    std::os::unix::fs::symlink(&outside, directory.join("checkpoints"))
        .expect("the link is creatable");
    let removed = delete_records(&root, &job("job-swapped"));
    assert!(removed.is_ok(), "the records go: {removed:?}");
    assert!(!directory.exists(), "the job's records are gone");
    assert_eq!(
        fs::read_to_string(outside.join("keep")).expect("the file outside is still there"),
        "kept",
        "the folder the link named is untouched"
    );
    let _ = fs::remove_dir_all(root);
}

/// A leftover tombstone a later hand replaced with a link to a folder outside:
/// the sweep finishes the name and never the folder it points at.
#[cfg(unix)]
#[test]
fn a_tombstone_swapped_for_a_link_is_unlinked_and_never_entered() {
    let root = scratch("tombstone-link");
    let outside = outside(&root);
    std::os::unix::fs::symlink(&outside, root.join(".deleting-job-stopped-7"))
        .expect("the leftover is creatable");
    records(&root, "job-next");
    let removed = delete_records(&root, &job("job-next"));
    assert!(removed.is_ok(), "the job goes: {removed:?}");
    assert_eq!(
        fs::read_to_string(outside.join("keep")).expect("the file outside is still there"),
        "kept",
        "the folder the leftover pointed at is untouched"
    );
    assert!(
        !root.join(".deleting-job-stopped-7").exists(),
        "the leftover's name is gone"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_job_listing_never_shows_a_tombstoned_job() {
    let root = scratch("listing");
    over_bound(&root, "job-bound");
    assert!(
        delete_records(&root, &job("job-bound")).is_err(),
        "the removal stops at the bound"
    );
    // A listing of the store's root shows a job's own name and never the name a
    // deletion moved it to.
    let listed: Vec<String> = names(&root)
        .into_iter()
        .filter(|name| kind(name) == Kind::Job)
        .collect();
    assert!(listed.is_empty(), "a tombstone is not a job: {listed:?}");
    let left = names(&root);
    assert_eq!(left.len(), 1, "the tombstone is all that is left: {left:?}");
    assert_eq!(
        kind(&left[0]),
        Kind::Tombstone,
        "the leftover is read as a tombstone: {left:?}"
    );
    assert!(
        !present(&root, &job("job-bound")),
        "a lookup never resolves the tombstone's name"
    );
    let _ = fs::remove_dir_all(root);
}
