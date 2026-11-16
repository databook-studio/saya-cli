//! The helper's `[tools] allow` gate on every entry point of `ToolExecutor`.
//!
//! The loop dispatches through `execute_with_context`, but the trait has two
//! more doors — the required `execute` and the `execute_with_result_cap`
//! default, which `ChildTools` overrides — and the helper's own members
//! (`checklist`) are served on the `execute` path. A gate on one door only is
//! a gate the advertisement can drift from, so these tests hold all three
//! against the same narrowed list.

use super::child_toolset;
use super::tests::{Sandbox, checkout, implementer_view};
use saya_agent::{CancellationToken, ToolError, ToolExecutionContext, ToolExecutor};

/// A whole-list `checklist` call: the shape the helper's own member serves.
fn checklist_call() -> serde_json::Value {
    serde_json::json!({"tasks": [{"title": "read the notes", "status": "pending"}]})
}

/// The three doors the loop and the trait offer, in one place.
async fn refusals(tools: &super::ChildTools, name: &str) -> Vec<(&'static str, ToolError)> {
    let arguments = checklist_call();
    let context = ToolExecutionContext {
        result_cap: 64 * 1024,
        cancellation: CancellationToken::new(),
    };
    vec![
        (
            "execute",
            tools
                .execute(name, arguments.clone())
                .await
                .expect_err("a removed name must refuse"),
        ),
        (
            "execute_with_result_cap",
            tools
                .execute_with_result_cap(name, arguments.clone(), 64 * 1024)
                .await
                .expect_err("a removed name must refuse"),
        ),
        (
            "execute_with_context",
            tools
                .execute_with_context(name, arguments, context)
                .await
                .expect_err("a removed name must refuse"),
        ),
    ]
}

/// The helper composes `checklist` for itself; a list that does not name it
/// takes it away, and every entry point refuses the call as an unknown tool —
/// the same refusal a member this surface never composed gets.
#[tokio::test]
async fn a_removed_name_is_refused_at_every_entry_point() {
    let checkout = checkout("allow-gate");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("allow-gate");
    let pair = sandbox.pair(&checkout);
    let (definitions, mut tools) = child_toolset(&view, &pair).unwrap();
    assert!(
        definitions
            .iter()
            .any(|definition| definition.name == "checklist"),
        "the helper composes `checklist` for itself, so this test has something to remove"
    );
    tools.narrow_to(
        saya_config::ResolvedTools::from_stated(Some(&["read".to_string()])).expect("a valid list"),
    );

    for (door, refused) in refusals(&tools, "checklist").await {
        assert!(
            matches!(refused, ToolError::UnsupportedTool),
            "{door} must refuse a name the list removed, not serve it: {refused:?}"
        );
        assert_eq!(
            refused.to_string(),
            "unsupported read-only tool",
            "{door} refuses with the same wording as an uncomposed member"
        );
    }
}

/// The gate is the list's, not a blanket refusal: with the list unstated the
/// same member serves the same call on every door.
#[tokio::test]
async fn the_same_name_is_served_when_the_list_keeps_it() {
    let checkout = checkout("allow-gate-open");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("allow-gate-open");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    assert!(tools.execute("checklist", checklist_call()).await.is_ok());
    assert!(
        tools
            .execute_with_result_cap("checklist", checklist_call(), 64 * 1024)
            .await
            .is_ok()
    );
}
