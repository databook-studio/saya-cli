//! Tests for the child no-effect classification: which write/edit refusals
//! provably changed nothing, and the executor wrap at admission. The truth
//! table pins the exact refusal set; the executor test pins that a write
//! refused at an inadmissible path reaches the model as a proven no-effect
//! call.

use super::child_toolset;
use super::no_effect::proves_no_effect;
use super::tests::{Sandbox, checkout, implementer_view};
use saya_agent::{ToolError, ToolExecutor};

fn write_call(path: &str, content: &str) -> serde_json::Value {
    serde_json::json!({"path": path, "content": content})
}

fn no_effect_refusals() -> Vec<ToolError> {
    vec![
        ToolError::WorkspaceEditNoMatch {
            path: "a/ok.txt".into(),
            matches: 0,
            size: 3,
            digest: "d".into(),
            hint: String::new(),
        },
        ToolError::WorkspaceEditAmbiguous {
            path: "a/ok.txt".into(),
            matches: 2,
            lines: vec![1, 5],
            size: 3,
            digest: "d".into(),
        },
        ToolError::WorkspaceEditMoved {
            path: "a/ok.txt".into(),
            expected_size: Some(3),
            current_size: 4,
            expected_digest: Some("d".into()),
            current_digest: "e".into(),
        },
        ToolError::WorkspaceEditEmptyAnchor {
            path: "a/ok.txt".into(),
        },
        ToolError::WorkspaceEditTooLarge {
            path: "a/ok.txt".into(),
            limit: 8,
            found: 9,
        },
        ToolError::WorkspaceEditNotText {
            path: "a/ok.txt".into(),
        },
        ToolError::WorkspaceAppendOffset {
            path: "a/ok.txt".into(),
            expected_offset: 1,
            current_size: 2,
            current_digest: "d".into(),
        },
        ToolError::WorkspaceWriteTooLarge { limit: 8 },
        ToolError::WorkspaceDigestRequired {
            operation: "edit",
            path: "a/ok.txt".into(),
            size: 3,
            current_digest: "d".into(),
        },
        ToolError::WorkspaceWriteMoved {
            path: "a/ok.txt".into(),
            expected_digest: "d".into(),
            current_size: 4,
            current_digest: "e".into(),
        },
        ToolError::PathNotString,
        ToolError::ContentNotString,
        ToolError::ArgumentsNotObject,
        ToolError::UnsupportedProperty,
        ToolError::ExpectedSizeNotUint,
        ToolError::ExpectedDigestNotString,
        ToolError::ReplaceAllNotBool,
        ToolError::ReplaceAllConflict,
        ToolError::UnsupportedTool,
        ToolError::WorkspaceUnavailable,
    ]
}

#[test]
fn the_listed_pre_write_refusals_prove_no_effect() {
    for error in no_effect_refusals() {
        assert!(
            proves_no_effect(&error),
            "expected a no-effect proof, got ambiguity: {error}"
        );
    }
}

#[test]
fn refusals_that_may_have_landed_do_not_prove_no_effect() {
    for error in [
        ToolError::Workspace("x".into()),
        ToolError::WorkspaceWrite("x".into()),
        ToolError::WorkspaceEdit("x".into()),
        ToolError::WorkspaceWrite(
            "recovery is enabled but this edit cannot be checkpointed (file too large); the file was not changed by someone else".into(),
        ),
        ToolError::WorkspaceWrite("credential path".into()),
        ToolError::WorkspaceWrite("secret-shaped content".into()),
        ToolError::WorkspaceWrite("database export".into()),
        ToolError::Runner("x".into()),
        ToolError::Fetch("x".into()),
    ] {
        assert!(
            !proves_no_effect(&error),
            "the effect may have landed and must stay uncertain: {error}"
        );
    }
}

#[test]
fn exact_checkpoint_quota_refusals_prove_no_effect() {
    let refusals = [
        "recovery is enabled but this edit cannot be checkpointed (file too large); the file was not changed",
        "recovery is enabled but this edit cannot be checkpointed (too many checkpoints); the file was not changed",
        "recovery is enabled but this edit cannot be checkpointed (session recovery storage is full); the file was not changed",
    ];
    for text in refusals {
        for error in [
            ToolError::WorkspaceWrite(text.into()),
            ToolError::WorkspaceEdit(text.into()),
        ] {
            assert!(
                proves_no_effect(&error),
                "expected exact pre-write checkpoint refusal to prove no effect: {error}"
            );
        }
    }
}

#[test]
fn exact_workspace_edit_constant_refusals_prove_no_effect() {
    let refusal = super::super::workspace::MULTI_EDIT_OVERLAP_REFUSAL;
    assert!(proves_no_effect(&ToolError::WorkspaceEdit(refusal.into())));
    assert!(!proves_no_effect(&ToolError::WorkspaceEdit(format!(
        "{refusal} "
    ))));
}

/// There is no path ceiling, so the admission refusal is about the path
/// shape: a path the workspace cannot hold is refused before the mutation,
/// and the model reads the original text with a proven no-effect call — a
/// retry of the same call changes nothing.
#[tokio::test]
async fn a_write_refused_at_an_inadmissible_path_proves_no_effect() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("no-effect-admission");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let refused = tools
        .execute("write", write_call("../escape.txt", "x"))
        .await
        .unwrap_err();

    assert!(refused.proves_no_effect());
    assert!(
        refused
            .to_string()
            .contains("the path is not an admissible workspace path"),
        "the model still reads the original refusal text: {refused}"
    );
    assert!(!sandbox.outer.join("escape.txt").exists());
}

#[tokio::test]
async fn non_string_write_and_edit_paths_prove_no_effect() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout);
    let sandbox = Sandbox::new("no-effect-non-string-path");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    for (name, arguments) in [
        ("write", serde_json::json!({"path": 3, "content": "x"})),
        (
            "edit",
            serde_json::json!({"path": 3, "offset": 0, "chunk": "x"}),
        ),
    ] {
        let refused = tools.execute(name, arguments).await.unwrap_err();
        assert!(
            refused.proves_no_effect(),
            "a malformed path is rejected before mutation: {refused}"
        );
    }
}
