use super::*;
use saya_harness::workspace::RemovalRefusal;
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    task::Poll,
    time::SystemTime,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "saya-delete-worker-{}-{stamp}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("victim.rs"), b"ordinary source\n").unwrap();
        fs::write(path.join("gate.rs"), b"publication gate\n").unwrap();
        Self(path)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn identity(workspace: &Workspace, path: &str) -> Sha256Digest {
    Sha256Digest::parse(&workspace.read(path, 0).unwrap().digest).unwrap()
}

// This polls the real production future through spawn_blocking, while a real
// prepared removal holds the cooperative gate. No callback supplies certainty.
#[tokio::test]
async fn dropped_delete_cancels_dispatched_work() {
    let root = Root::new();
    let workspace = Workspace::open(&root.0).unwrap();
    let digest = identity(&workspace, "victim.rs");
    let held = workspace
        .prepare_guarded_remove(
            "gate.rs",
            identity(&workspace, "gate.rs").as_str(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    let token = CancellationToken::new();
    let mut deletion = Box::pin(run(
        workspace,
        WorkspacePath::try_from("victim.rs".to_owned()).unwrap(),
        digest,
        token.clone(),
    ));
    assert!(matches!(
        futures_util::poll!(deletion.as_mut()),
        Poll::Pending
    ));
    drop(deletion);
    assert_eq!(token.reason(), Some(CancellationReason::OwnerFailure));
    drop(held);
    assert_eq!(
        fs::read(root.0.join("victim.rs")).unwrap(),
        b"ordinary source\n"
    );
}

#[tokio::test]
async fn caller_cancellation_reaches_the_dispatched_publication_owner() {
    let root = Root::new();
    let workspace = Workspace::open(&root.0).unwrap();
    let digest = identity(&workspace, "victim.rs");
    let held = workspace
        .prepare_guarded_remove(
            "gate.rs",
            identity(&workspace, "gate.rs").as_str(),
            CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
    let token = CancellationToken::new();
    let mut deletion = Box::pin(run(
        workspace,
        WorkspacePath::try_from("victim.rs".to_owned()).unwrap(),
        digest,
        token.clone(),
    ));
    assert!(matches!(
        futures_util::poll!(deletion.as_mut()),
        Poll::Pending
    ));
    token.cancel();
    drop(held);
    assert!(matches!(
        deletion.await,
        Err(ControlledRemoveError::NoEffect(RemovalRefusal::Cancelled))
    ));
    assert_eq!(token.reason(), Some(CancellationReason::User));
    assert!(root.0.join("victim.rs").exists());
}

#[tokio::test]
async fn successful_join_confirms_unlink_without_cancelling_the_caller() {
    let root = Root::new();
    let workspace = Workspace::open(&root.0).unwrap();
    let digest = identity(&workspace, "victim.rs");
    let token = CancellationToken::new();
    run(
        workspace,
        WorkspacePath::try_from("victim.rs".to_owned()).unwrap(),
        digest,
        token.clone(),
    )
    .await
    .unwrap();
    assert!(!root.0.join("victim.rs").exists());
    assert!(!token.is_cancelled());
}
