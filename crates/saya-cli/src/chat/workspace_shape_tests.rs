//! The shape a session decides before it prepares a run.
//!
//! Three answers, each read from the folder itself: a folder no run leases is
//! writable, a folder whose lease another live owner holds reads only, and a
//! folder that cannot be a root is no workspace at all.

#![cfg(unix)]

use super::workspace_shape::{WorkspaceShape, decide};
use saya_harness::coding::local::LocalRoot;
use std::{
    fs,
    path::{Path, PathBuf},
    process,
    time::{SystemTime, UNIX_EPOCH},
};

/// The session these cases run as: no case's folder is leased for it.
const SESSION: &str = "session-shape-case";

/// A throwaway directory removed when the case ends.
struct Scratch(PathBuf);

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One case's folder and the state root a run would keep its records in, both
/// canonical, both outside each other.
fn case(name: &str) -> (Scratch, PathBuf, PathBuf) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let base = std::env::temp_dir().join(format!("saya-shape-{name}-{}-{stamp}", process::id()));
    let folder = base.join("site");
    let state = base.join("state");
    fs::create_dir_all(&folder).unwrap();
    fs::create_dir_all(&state).unwrap();
    let base = base.canonicalize().unwrap();
    (Scratch(base.clone()), base.join("site"), base.join("state"))
}

/// Holds the folder's lease the way a second session does: one more open
/// descriptor on the same lock file, taken with the same non-blocking `flock`
/// a run takes.
fn hold_lease(state_root: &Path, folder: &Path) -> fs::File {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;

    let root = LocalRoot::open(folder, state_root).expect("the folder is an in-place root");
    let directory = state_root.join("inplace-leases");
    fs::create_dir_all(&directory).unwrap();
    let path = directory.join(format!("{}.lock", root.id().as_str()));
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&path)
        .expect("the lease file opens");
    // SAFETY: `file` owns a valid descriptor for the duration of the call.
    let locked = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(locked, 0, "the test takes its own lease");
    file
}

/// A folder no run leases is a folder a run may write.
#[test]
fn a_free_folder_is_writable() {
    let (_scratch, folder, state) = case("free");
    assert_eq!(decide(&folder, &state, SESSION), WorkspaceShape::Writable);
}

/// A folder whose lease another live owner holds reads only, and the reason
/// names that other session.
#[test]
fn a_leased_folder_reads_only_and_the_reason_names_the_other_session() {
    let (_scratch, folder, state) = case("leased");
    let held = hold_lease(&state, &folder);
    assert_eq!(
        decide(&folder, &state, SESSION),
        WorkspaceShape::ReadOnlyFiles {
            reason: "another session owns this folder".to_owned()
        }
    );
    // The lease the test gave back is not an owner any more.
    drop(held);
    assert_eq!(decide(&folder, &state, SESSION), WorkspaceShape::Writable);
}

/// The filesystem root, the home directory and saya's own state directory are
/// no run's workspace, and neither is a folder that is not there.
#[test]
fn a_root_that_cannot_be_a_workspace_is_refused() {
    let (_scratch, _folder, state) = case("refused");
    let home = std::env::var_os("HOME").expect("this platform states a home directory");
    let home = PathBuf::from(home)
        .canonicalize()
        .expect("home canonicalizes");
    let absent = state.join("absent");
    for (path, expected) in [
        (Path::new("/"), "filesystem root"),
        (home.as_path(), "home directory"),
        (state.as_path(), "state directory"),
        (absent.as_path(), "does not exist"),
    ] {
        let WorkspaceShape::NoWorkspace { reason } = decide(path, &state, SESSION) else {
            panic!(
                "{} must be no workspace: {:?}",
                path.display(),
                decide(path, &state, SESSION)
            );
        };
        assert!(
            reason.contains(expected),
            "the reason for {} names {expected:?}: {reason}",
            path.display()
        );
    }
}
