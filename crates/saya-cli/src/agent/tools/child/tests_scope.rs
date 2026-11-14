//! The in-place `--write` scope at the child tool boundary: a path outside the
//! approved set is refused before any anchored mutation, and `.` covers the
//! whole folder.

use std::fs;

use super::tests::{Sandbox, checkout, implementer_view};
use super::*;
use saya_agent::{ToolError, ToolExecutor};
use saya_harness::coding::integration::AssignedPaths;
use serde_json::json;

fn scope(entries: &[&str]) -> AssignedPaths {
    AssignedPaths::parse(entries.iter().map(|entry| (*entry).to_owned())).unwrap()
}

#[tokio::test]
async fn in_place_write_outside_scope_refused() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("inplace-scope-refused");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();
    let tools = tools.with_write_scope(Some(scope(&["src"])));

    for (name, arguments) in [
        (
            "write",
            json!({"path":"outside.txt","content":"must not exist"}),
        ),
        (
            "edit",
            json!({"path":"outside.txt","old_text":"a","new_text":"b"}),
        ),
    ] {
        let outcome = tools.execute(name, arguments).await;
        assert!(
            matches!(
                &outcome,
                Err(ToolError::NoEffect(inner))
                    if matches!(inner.as_ref(), ToolError::WorkspaceWrite(text)
                        if text.contains("outside this job's write scope")
                            && text.contains("it may write src")
                            && text.contains("report what you need"))
            ),
            "{name} outside the scope must name what it may write: {outcome:?}"
        );
    }
    assert!(!sandbox.ws_root().join("outside.txt").exists());
}

#[test]
fn the_child_decider_names_the_scope_it_will_admit() {
    use saya_agent::ApprovalDecider;

    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("inplace-scope-decider");
    let pair = sandbox.pair(&checkout);
    let (definitions, _) = child_toolset(&view, &pair).unwrap();
    let write = definitions
        .iter()
        .find(|definition| definition.name == "write")
        .expect("the implementer advertises write");
    let approval = super::decider::ExactWriteChildApproval::new(&view, pair.checkout_id().clone())
        .with_write_scope(Some(scope(&["src", "tests"])));

    let detail = approval
        .refusal_detail(write, &json!({"path":"README.md","content":"x"}))
        .expect("a path outside the scope is refused with a reason");
    assert!(detail.contains("it may write src, tests"), "{detail}");
    assert!(detail.contains("report what you need"), "{detail}");
}

#[tokio::test]
async fn a_whole_folder_scope_still_refuses_a_workspace_escape() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("inplace-scope-escape");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();
    let tools = tools.with_write_scope(Some(scope(&["."])));

    let outcome = tools
        .execute(
            "write",
            json!({"path":"../escape.txt","content":"must not exist"}),
        )
        .await;
    assert!(outcome.is_err(), "an escape is refused: {outcome:?}");
    assert!(!sandbox.outer.join("escape.txt").exists());
}

#[tokio::test]
async fn in_place_whole_folder_scope_admits_all() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("inplace-scope-whole");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();
    let tools = tools.with_write_scope(Some(scope(&["."])));

    for (path, content) in [("top.txt", "one\n"), ("nested.txt", "two\n")] {
        tools
            .execute("write", json!({"path":path,"content":content}))
            .await
            .unwrap_or_else(|error| panic!("{path} must be admitted: {error:?}"));
        assert_eq!(
            fs::read(sandbox.ws_root().join(path)).unwrap(),
            content.as_bytes()
        );
    }
}
