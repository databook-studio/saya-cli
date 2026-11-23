//! The `read` tool's privacy refusal: the one rule the file tools share, stated
//! before a byte of the file is read — and the sentence a mention in the
//! composer is refused with, because it is this tool's read a mention performs.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use saya_agent::{ToolError, ToolExecutor};
use saya_harness::workspace::Workspace;

use super::*;

/// A workspace under `./target`, removed when it drops.
struct Sandbox {
    root: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let root =
            Path::new("target").join(format!("tmp-read-private-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).expect("sandbox directory must create");
        let ws = Workspace::open(&root).expect("workspace root must open");
        Self { root, ws }
    }

    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }

    fn write(&self, path: &str, bytes: &[u8]) {
        fs::write(self.root.join(path), bytes).expect("fixture file must write");
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// A credential-shaped path is refused whole, with the refusal naming it — and
/// its bytes are never served.
#[tokio::test]
async fn read_refuses_a_private_path_with_its_own_sentence() {
    let sandbox = Sandbox::new("refused");
    sandbox.write(".env", b"TOKEN=placeholder\n");
    sandbox.write("id_rsa", b"placeholder key material\n");
    sandbox.write("src/main.rs", b"fn main() {}\n");
    let tools = sandbox.tools();
    for path in [".env", "id_rsa"] {
        let refused = tools
            .execute("read", serde_json::json!({ "path": path }))
            .await
            .expect_err("a private path is refused, never served");
        let sentence = refused.to_string();
        assert!(
            matches!(refused, ToolError::WorkspacePrivatePath { .. }),
            "{path} is refused as the privacy rule refuses it: {refused:?}"
        );
        assert!(
            sentence.contains(path),
            "the refusal names the path it refused: {sentence}"
        );
        assert!(
            !sentence.contains("placeholder"),
            "nothing of the file's content is echoed: {sentence}"
        );
    }
    // The rule is the shape, not "anything unusual": ordinary source still
    // reads.
    let read = tools
        .execute("read", serde_json::json!({ "path": "src/main.rs" }))
        .await
        .expect("ordinary source reads");
    assert!(
        read["content"]
            .as_str()
            .unwrap_or_default()
            .contains("fn main")
    );
}
