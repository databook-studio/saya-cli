//! E4 RED slice: `workspace_read` returns a whole-file digest, and a large
//! file is edited (grep → edit) without any whole-file model read. These
//! tests are written first and MUST FAIL against the current code: there is
//! no `digest` field on the read result yet, and no digest-capable path that
//! serves a digest without serving the whole file.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::ToolExecutor;
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_READ_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsdigest-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    fn ws_root(&self) -> PathBuf {
        self.outer.join("ws")
    }

    /// Database tools with the sandbox workspace attached and no connections.
    fn tools(&self) -> DatabaseTools {
        DatabaseTools::with_registry(
            crate::connection::ConnectionRegistry::new("primary"),
            100,
            true,
            None,
        )
        .with_workspace(Some(Arc::new(self.ws.clone())))
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.outer);
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(64);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

#[path = "workspace_read_digest_tests/digest_and_bounds.rs"]
mod digest_and_bounds;
#[path = "workspace_read_digest_tests/read_and_edit.rs"]
mod read_and_edit;
