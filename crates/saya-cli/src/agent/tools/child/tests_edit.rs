//! Tests for the edit admission effect: a `workspace_edit` call must only
//! touch files at admitted paths. Authored, not executed: saya-cli depends on
//! DuckDB and is never built in this environment.

use std::fs;

use super::child_toolset;
use super::tests::{Sandbox, checkout, implementer_view};
use saya_agent::{ToolError, ToolExecutor};

fn edit_call(path: &str) -> serde_json::Value {
    serde_json::json!({"path": path, "offset": 0, "chunk": "hi"})
}

// authored, not executed
#[tokio::test]
async fn edit_append_creates_only_at_an_admitted_path() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("edit-append");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let admitted = tools.execute("workspace_edit", edit_call("a/ok.txt")).await;
    assert!(admitted.is_ok());
    let created = sandbox.ws_root().join("a/ok.txt");
    assert!(created.exists());
    assert_eq!(fs::read_to_string(created).unwrap(), "hi");

    let refused = tools
        .execute("workspace_edit", edit_call("a/other.txt"))
        .await;
    assert!(matches!(refused, Err(ToolError::WorkspaceWrite(_))));
    assert!(!sandbox.ws_root().join("a/other.txt").exists());
}

// authored, not executed
#[tokio::test]
async fn edit_without_write_paths_is_unsupported() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &[]);
    let sandbox = Sandbox::new("edit-absent");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let outcome = tools.execute("workspace_edit", edit_call("a/ok.txt")).await;
    assert!(matches!(outcome, Err(ToolError::UnsupportedTool)));
}

// authored, not executed
#[tokio::test]
async fn edit_with_a_second_path_key_is_refused() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("edit-second-key");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let outcome = tools
        .execute(
            "workspace_edit",
            serde_json::json!({
                "path": "a/ok.txt",
                "offset": 0,
                "chunk": "hi",
                "paths": ["a/other.txt"]
            }),
        )
        .await;
    assert!(outcome.is_err());
    assert!(!sandbox.ws_root().join("a/other.txt").exists());
}
