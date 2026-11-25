#[path = "worktree_inventory/compatibility.rs"]
mod compatibility;
#[path = "worktree_inventory/deadline.rs"]
mod deadline;
#[path = "worktree_inventory/identity.rs"]
mod identity;
#[path = "worktree_inventory/ignored_outputs.rs"]
mod ignored_outputs;
#[path = "worktree_inventory/metadata.rs"]
mod metadata;
#[path = "worktree_inventory/refusals.rs"]
mod refusals;
#[path = "worktree_inventory/status.rs"]
mod status;
#[path = "worktree_inventory_support.rs"]
mod support;

/// A wrapper is exec-safe by construction: the write waits until no process
/// holds it for writing. A child forked by another test thread inherits a copy
/// of an open write descriptor until it execs, so an immediate exec of the
/// just-written wrapper fails with ETXTBSY on Linux. This test holds that
/// descriptor itself, which makes the race deterministic rather than
/// scheduling-dependent.
#[cfg(target_os = "linux")]
#[test]
fn a_wrapper_written_while_a_write_descriptor_is_open_is_exec_safe() {
    use std::{process::Command, thread, time::Duration};

    let temp = support::TempRoot::new("exec-ready-wrapper");
    let real_git = support::git_executable();
    let wrapper_path = temp.path().join("git-version-wrapper");
    let held = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(&wrapper_path)
        .unwrap();
    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(100));
        drop(held);
    });

    // Without the readiness wait this returns while the descriptor is still
    // open, and the exec below fails with ETXTBSY.
    let (wrapper, _log) = support::versioned_git(temp.path(), &real_git, "2.50.1", false);
    let output = Command::new(&wrapper)
        .arg("--version")
        .output()
        .expect("the wrapper must exec");
    assert!(output.status.success(), "the wrapper must run: {output:?}");

    releaser.join().unwrap();
}
