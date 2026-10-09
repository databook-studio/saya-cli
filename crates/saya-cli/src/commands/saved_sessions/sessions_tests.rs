//! Unit tests for the shared saved-session operations: the listing shape
//! (order, sizes, warning-band label, pending names, bound), and every
//! delete outcome — including the file-system effects each one must and
//! must not have.

use super::*;
use saya_store::{
    FsSessionStore, RedactedMessage, RedactedSession, SESSION_WARN_BYTES, SessionCapacityStatus,
    SessionStore,
};
use std::path::PathBuf;
use std::time::Duration;

fn block_on<T>(future: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime")
        .block_on(future)
}

fn scratch(label: &str) -> PathBuf {
    let base = std::env::temp_dir().join(format!(
        "saya-cli-sessions-unit-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("sessions")).unwrap();
    // The tombstone lives in the store root's PARENT (`<root>/../lifecycle`),
    // so a store root that is a direct child of the temp dir shares ONE
    // lifecycle directory with every other such test — a parallel delete's
    // tombstone appeared in another test's "empty store" listing. Hand back
    // the nested `sessions` dir: the base then owns `lifecycle/` and
    // `state/` for this store root alone.
    base.join("sessions")
}

fn dir_names(path: &std::path::Path) -> Vec<String> {
    let mut names = std::fs::read_dir(path)
        .unwrap()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn pinned_mtime(path: &std::path::Path, ago: Duration) {
    let mtime = std::time::SystemTime::now().checked_sub(ago).unwrap();
    let file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.set_modified(mtime).unwrap();
}

struct RestorePermissions(std::path::PathBuf);

impl Drop for RestorePermissions {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o700));
        }
    }
}

include!("sessions_tests/listing.rs");

include!("sessions_tests/delete.rs");

include!("sessions_tests/cleanup_retry.rs");

include!("sessions_tests/root_isolation.rs");
