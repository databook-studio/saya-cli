//! Tests for the edit admission effect: a `edit` call must stay inside the
//! workspace a write-capable view holds: any path in the shared working
//! checkout.

use std::fs;

use super::child_toolset;
use super::tests::{Sandbox, checkout, implementer_view, read_only_view};
use saya_agent::{ToolError, ToolExecutor};

fn edit_call(path: &str) -> serde_json::Value {
    serde_json::json!({"path": path, "offset": 0, "chunk": "hi"})
}

/// There is no path ceiling: a write-capable child edits any path in the
/// shared working checkout; only a path the workspace cannot hold is refused.
// authored, not executed
#[tokio::test]
async fn edit_creates_at_any_path_inside_the_workspace() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("edit-append");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    for path in ["a/ok.txt", "a/other.txt"] {
        let edited = tools.execute("edit", edit_call(path)).await;
        assert!(edited.is_ok(), "{path}: {edited:?}");
        let created = sandbox.ws_root().join(path);
        assert!(created.exists());
        assert_eq!(fs::read_to_string(created).unwrap(), "hi");
    }

    let refused = tools.execute("edit", edit_call("../escape.txt")).await;
    assert!(matches!(
        refused,
        Err(ToolError::NoEffect(inner))
            if matches!(inner.as_ref(), ToolError::WorkspaceWrite(_))
    ));
    assert!(!sandbox.outer.join("escape.txt").exists());
}

// authored, not executed
#[tokio::test]
async fn edit_without_the_write_capability_is_unsupported() {
    let checkout = checkout("checkout");
    let view = read_only_view(&checkout);
    let sandbox = Sandbox::new("edit-absent");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let outcome = tools.execute("edit", edit_call("a/ok.txt")).await;
    assert!(matches!(
        outcome,
        Err(ToolError::NoEffect(inner))
            if matches!(inner.as_ref(), ToolError::UnsupportedTool)
    ));
}

/// A helper with no write capability is offered no notebook edit either: the
/// definition is hidden from a read-only surface, not merely denied at the
/// door, so the model is never invited to call it.
// authored, not executed
#[tokio::test]
async fn notebook_edit_without_the_write_capability_is_unsupported() {
    let checkout = checkout("checkout");
    let view = read_only_view(&checkout);
    let sandbox = Sandbox::new("notebook-edit-absent");
    let pair = sandbox.pair(&checkout);
    let (definitions, tools) = child_toolset(&view, &pair).unwrap();

    assert!(
        !definitions
            .iter()
            .any(|definition| definition.name == "notebook_edit"),
        "a read-only helper advertises no notebook_edit"
    );
    let outcome = tools
        .execute(
            "notebook_edit",
            serde_json::json!({"path": "a/ok.ipynb", "mode": "delete", "index": 0}),
        )
        .await;
    assert!(matches!(
        outcome,
        Err(ToolError::NoEffect(inner))
            if matches!(inner.as_ref(), ToolError::UnsupportedTool)
    ));
}

// authored, not executed
#[tokio::test]
async fn edit_with_a_second_path_key_is_refused() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("edit-second-key");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let outcome = tools
        .execute(
            "edit",
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

// authored, not executed
#[tokio::test]
async fn child_write_definitions_require_the_child_decider() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("edit-decider");
    let pair = sandbox.pair(&checkout);
    let (definitions, _) = child_toolset(&view, &pair).unwrap();

    let write = definitions
        .iter()
        .find(|d| d.name == "write")
        .expect("write is defined for a write-capable view");
    assert!(write.effect.requires_approval);
    let edit = definitions
        .iter()
        .find(|d| d.name == "edit")
        .expect("edit is defined for a write-capable view");
    assert!(edit.effect.requires_approval);
    let read = definitions
        .iter()
        .find(|d| d.name == "read")
        .expect("read is always defined");
    assert!(!read.effect.requires_approval);
}
