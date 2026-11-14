use super::*;
use saya_harness::workspace::Workspace;
use saya_store::FsSessionStore;
use std::{fs, path::PathBuf, sync::Arc, sync::atomic::AtomicBool};

use super::super::super::checkpoint::CheckpointRecorder;

struct Sandbox {
    outer: PathBuf,
    workspace: Arc<Workspace>,
}

impl Sandbox {
    fn new() -> Self {
        let outer = std::env::temp_dir().join(format!("saya-multi-cancel-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(&outer).expect("sandbox directory must create");
        let workspace_root = outer.join("workspace");
        fs::create_dir_all(&workspace_root).expect("workspace root must create");
        let workspace = Arc::new(Workspace::open(&workspace_root).unwrap());
        Self { outer, workspace }
    }

    fn tools(&self, store: &Arc<FsSessionStore>) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::clone(&self.workspace)))
        .with_checkpoints(Some(CheckpointRecorder::new(
            Arc::clone(store),
            "session".into(),
            Arc::new(AtomicBool::new(true)),
        )))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

#[tokio::test]
async fn cancelled_multi_edit_prepares_then_cleanly_settles_its_checkpoint() {
    let sandbox = Sandbox::new();
    let original = b"before needle after";
    sandbox.workspace.write("notes.txt", original).unwrap();
    let store = Arc::new(FsSessionStore::new(sandbox.outer.join("sessions")));
    let tools = sandbox.tools(&store);
    let cancellation = CancellationToken::new();
    cancellation.cancel();

    let result = tools
        .workspace_multi_edit(
            "notes.txt",
            &[("needle".into(), "changed".into())],
            Some(original.len() as u64),
            Some(&hex_digest(original)),
            &cancellation,
        )
        .await;

    assert!(matches!(result, Err(ToolError::NoEffect(_))));
    assert_eq!(
        fs::read(sandbox.outer.join("workspace/notes.txt")).unwrap(),
        original
    );
    assert!(
        sandbox.outer.join("sessions/session/checkpoints").is_dir(),
        "the enabled real recorder prepared its checkpoint directory"
    );
    assert!(
        store
            .list_checkpoints("session")
            .expect("list prepared checkpoints")
            .is_empty(),
        "the known-no-effect cancellation settles and removes the prepared record"
    );
}
