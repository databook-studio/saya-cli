//! Tests for the `workspace_read` tool: the one contained, bounded file read
//! the model's tools can perform. The defining invariants: a path outside the
//! workspace surfaces as a typed, model-readable `ToolError` (never a panic,
//! never a silent empty result), and the byte bound truncates with the
//! truncation reported in the result rather than silently dropping bytes.

use std::{fs, path::PathBuf, sync::Arc};

use super::*;
use saya_agent::{LocalStateEffect, ToolError, ToolExecutor};
use saya_harness::workspace::Workspace;

use super::database_tools::WORKSPACE_READ_MAX_BYTES;

/// A sandbox workspace under the OS temp dir, removed on drop. The root sits
/// one level down (`outer/ws`) so a `../` escape has a real target
/// (`outer/outside.txt`) to name.
struct Sandbox {
    outer: PathBuf,
    ws: Workspace,
}

impl Sandbox {
    fn new(label: &str) -> Self {
        let outer =
            std::env::temp_dir().join(format!("saya-wsread-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&outer);
        fs::create_dir_all(outer.join("ws")).expect("sandbox directory must create");
        let ws = Workspace::open(&outer.join("ws")).expect("workspace root must open");
        Self { outer, ws }
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

#[tokio::test]
async fn workspace_read_returns_the_contained_content() {
    let sandbox = Sandbox::new("content");
    sandbox
        .ws
        .write("notes/summary.md", b"hello workspace")
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute(
            "workspace_read",
            serde_json::json!({"path": "notes/summary.md"}),
        )
        .await
        .expect("a contained read must succeed");
    assert_eq!(result["path"], "notes/summary.md");
    assert_eq!(result["content"], "hello workspace");
    assert_eq!(result["size"].as_u64(), Some(15));
    assert_eq!(result["truncated"], serde_json::Value::Bool(false));
}

/// The escape the containment layer exists for: a `..` path that rises above
/// the root is refused as a typed `ToolError` carrying the harness's own
/// reason — readable by the model, not a panic and not a silent empty result.
#[tokio::test]
async fn workspace_read_surfaces_a_path_outside_the_workspace_as_a_typed_error() {
    let sandbox = Sandbox::new("escape");
    fs::write(sandbox.outer.join("outside.txt"), b"secret").expect("plant must succeed");
    let tools = sandbox.tools();
    let error = tools
        .execute(
            "workspace_read",
            serde_json::json!({"path": "../outside.txt"}),
        )
        .await
        .expect_err("a path outside the workspace must be refused, not served");
    match error {
        ToolError::Workspace(message) => {
            assert!(
                message.contains("escapes the run workspace"),
                "the refusal must name the containment failure: {message}"
            );
            assert!(
                message.contains("../outside.txt"),
                "the refusal must name the refused path: {message}"
            );
        }
        other => panic!("expected a typed workspace error, got: {other}"),
    }
}

/// The byte bound is enforced and the truncation is reported: content is
/// capped at exactly `WORKSPACE_READ_MAX_BYTES`, `size` still reports the
/// file's full length, and `truncated` is true — never silent.
#[tokio::test]
async fn workspace_read_truncates_at_the_byte_bound_and_reports_it() {
    let sandbox = Sandbox::new("truncate");
    let total = WORKSPACE_READ_MAX_BYTES + 16;
    sandbox
        .ws
        .write("big.bin", &vec![b'a'; total as usize])
        .expect("contained write must succeed");
    let tools = sandbox.tools();
    let result = tools
        .execute("workspace_read", serde_json::json!({"path": "big.bin"}))
        .await
        .expect("a bounded read must succeed");
    assert_eq!(result["truncated"], serde_json::Value::Bool(true));
    assert_eq!(result["size"].as_u64(), Some(total));
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert_eq!(
        content.len() as u64,
        WORKSPACE_READ_MAX_BYTES,
        "content is capped at exactly the bound"
    );
    assert!(content.bytes().all(|byte| byte == b'a'));
}

/// No workspace attached (every current caller, until the run engine opens
/// one): the tool denies with a typed error instead of pretending to read.
#[tokio::test]
async fn workspace_read_denies_when_no_workspace_is_attached() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("workspace_read", serde_json::json!({"path": "notes.md"}))
        .await
        .expect_err("no workspace, no read");
    assert_eq!(error, ToolError::WorkspaceUnavailable);
    assert!(
        error.to_string().contains("no workspace is available"),
        "the denial must be readable by the model: {error}"
    );
}

/// The definition is advertised regardless of the data-sharing gate (it
/// touches no database data) and is read-shaped: read-only approval permits
/// it, and it declares `LocalStateEffect::Read`.
#[test]
fn workspace_read_definition_is_read_shaped_and_always_advertised() {
    let tools = DatabaseTools::definitions(false, false, false, false, false);
    let tool = tools
        .iter()
        .find(|tool| tool.name == "workspace_read")
        .expect("workspace_read is advertised even with the data gate closed");
    assert!(tool.read_only);
    assert!(!tool.effect.database_data);
    assert!(!tool.effect.external_side_effect);
    assert!(!tool.effect.requires_approval);
    assert_eq!(tool.effect.local_state, LocalStateEffect::Read);
    assert_eq!(tool.parameters["required"], serde_json::json!(["path"]));
    assert_eq!(tool.completion.as_deref(), Some("workspace file read"));
}

#[tokio::test]
async fn workspace_read_rejects_a_non_string_path() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute("workspace_read", serde_json::json!({"path": 7}))
        .await
        .expect_err("a non-string path must be rejected at validation");
    assert_eq!(error, ToolError::PathNotString);
}

#[tokio::test]
async fn workspace_read_rejects_unknown_arguments() {
    let tools = DatabaseTools::with_registry(
        crate::connection::ConnectionRegistry::new("primary"),
        100,
        true,
        None,
    );
    let error = tools
        .execute(
            "workspace_read",
            serde_json::json!({"path": "notes.md", "sql": "SELECT 1"}),
        )
        .await
        .expect_err("unknown arguments must be rejected");
    assert_eq!(error, ToolError::UnsupportedProperty);
}
