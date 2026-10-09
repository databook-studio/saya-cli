//! Tests for the workspace search tools: `workspace_list`, `glob`, and
//! `grep`. The defining invariants: with no workspace attached each tool
//! denies with a typed error; a path outside the workspace surfaces the
//! harness's own containment reason; every bound fails as a typed bounds
//! error rather than truncating quietly (a truncated "no matches" reads as
//! proof of absence); and a symlink — wherever it points — is neither
//! followed nor matched by the search.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{LocalStateEffect, ToolError, ToolExecutor};
use saya_harness::workspace::Workspace;

use super::database_tools::{
    WORKSPACE_GLOB_MAX_MATCHES, WORKSPACE_GLOB_MAX_VISITED, WORKSPACE_GREP_MAX_FILE_BYTES,
    WORKSPACE_GREP_MAX_LINE_BYTES, WORKSPACE_GREP_MAX_MATCHES, WORKSPACE_GREP_MAX_VISITED,
    WORKSPACE_LIST_MAX_ENTRIES,
};

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `..` escape and an outside symlink target
/// have real places to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wssearch-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
    }

    /// Plants many files directly (no per-file fsync): only the bulk-bound
    /// tests need more files than the contained write is comfortable with.
    fn plant(&self, names: impl Iterator<Item = String>, bytes: &[u8]) {
        for name in names {
            fs::write(self.ws.root().join(name), bytes).expect("plant must succeed");
        }
    }

    /// Database tools with the sandbox workspace attached and no connections —
    /// a workspace-only run has no selected profile.
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

// ---------------------------------------------------------------- list

include!("workspace_search_tests/list.rs");

// ---------------------------------------------------------------- glob

/// Runs one `glob` call and returns the matched paths.
async fn glob_paths(tools: &DatabaseTools, pattern: &str) -> Vec<String> {
    let result = tools
        .execute("glob", serde_json::json!({"pattern": pattern}))
        .await
        .expect("a contained glob must succeed");
    result["matches"]
        .as_array()
        .expect("matches array")
        .iter()
        .map(|value| value.as_str().expect("path string").to_string())
        .collect()
}

include!("workspace_search_tests/glob_matches.rs");

include!("workspace_search_tests/search_bounds.rs");

include!("workspace_search_tests/glob_arguments.rs");

// ---------------------------------------------------------------- grep

include!("workspace_search_tests/grep.rs");

// ---------------------------------------------------------- definitions

include!("workspace_search_tests/definitions.rs");
