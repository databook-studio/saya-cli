#![cfg(unix)]
use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct TempRoot(PathBuf);
impl TempRoot {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "saya-controlled-remove-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).expect("create only this unique test directory");
        Self(path)
    }
    fn path(&self) -> &Path {
        &self.0
    }
}
impl Drop for TempRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn target() -> (TempRoot, Workspace, String) {
    let temp = TempRoot::new();
    fs::write(temp.path().join("victim.rs"), b"ordinary source\n").unwrap();
    let workspace = Workspace::open(temp.path()).unwrap();
    let digest = workspace.read("victim.rs", 1024).unwrap().digest;
    (temp, workspace, digest)
}

// Real prepared owner state retains the cooperative gate and verified FD.
// External filesystem replacement requires no production test callback.
#[test]
fn prepared_removal_refuses_a_replaced_basename() {
    let (temp, workspace, digest) = target();
    let prepared = workspace
        .prepare_guarded_remove(
            "victim.rs",
            &digest,
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    fs::rename(
        temp.path().join("victim.rs"),
        temp.path().join("original.rs"),
    )
    .unwrap();
    fs::write(temp.path().join("victim.rs"), b"replacement source\n").unwrap();
    assert!(matches!(
        prepared.commit(),
        Err(ControlledRemoveError::NoEffect(RemovalRefusal::Changed))
    ));
    assert_eq!(
        fs::read(temp.path().join("victim.rs")).unwrap(),
        b"replacement source\n"
    );
    assert_eq!(
        fs::read(temp.path().join("original.rs")).unwrap(),
        b"ordinary source\n"
    );
}

#[test]
fn cancellation_after_prepare_refuses_before_unlink() {
    let (temp, workspace, digest) = target();
    let token = CancellationToken::new();
    let prepared = workspace
        .prepare_guarded_remove(
            "victim.rs",
            &digest,
            token.clone(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    token.cancel();
    assert!(matches!(
        prepared.commit(),
        Err(ControlledRemoveError::NoEffect(RemovalRefusal::Cancelled))
    ));
    assert_eq!(
        fs::read(temp.path().join("victim.rs")).unwrap(),
        b"ordinary source\n"
    );
}

#[test]
fn deadline_after_prepare_refuses_before_unlink() {
    let (temp, workspace, digest) = target();
    let deadline = Instant::now() + Duration::from_secs(2);
    let prepared = workspace
        .prepare_guarded_remove("victim.rs", &digest, CancellationToken::new(), deadline)
        .unwrap();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(
        prepared.commit(),
        Err(ControlledRemoveError::NoEffect(RemovalRefusal::Deadline))
    ));
    assert!(temp.path().join("victim.rs").is_file());
}

#[cfg(unix)]
mod refusals;
