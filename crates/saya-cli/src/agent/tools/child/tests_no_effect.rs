//! Tests for the child no-effect classification: which write/edit refusals
//! provably changed nothing, and the executor wrap at admission. The truth
//! table pins the exact refusal set; the executor test pins that an
//! unadmitted path reaches the model as a proven no-effect call.

use super::super::workspace::FILE_EXISTS_REQUIRES_DIGEST;
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
        ToolError::PathNotString,
        ToolError::ContentNotString,
        ToolError::ArgumentsNotObject,
        ToolError::UnsupportedProperty,
        ToolError::ExpectedSizeNotUint,
        ToolError::ExpectedDigestNotString,
        ToolError::UnsupportedTool,
        ToolError::WorkspaceUnavailable,
        ToolError::WorkspaceWrite(FILE_EXISTS_REQUIRES_DIGEST.to_owned()),
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
        ToolError::Runner("x".into()),
        ToolError::Fetch("x".into()),
    ] {
        assert!(
            !proves_no_effect(&error),
            "the effect may have landed and must stay uncertain: {error}"
        );
    }
}

#[tokio::test]
async fn a_write_refused_at_an_unadmitted_path_proves_no_effect() {
    let checkout = checkout("checkout");
    let view = implementer_view(&checkout, &["a/ok.txt"]);
    let sandbox = Sandbox::new("no-effect-admission");
    let pair = sandbox.pair(&checkout);
    let (_, tools) = child_toolset(&view, &pair).unwrap();

    let refused = tools
        .execute("workspace_write", write_call("a/other.txt", "x"))
        .await
        .unwrap_err();

    assert!(refused.proves_no_effect());
    assert!(
        refused
            .to_string()
            .contains("the child authority does not admit this write"),
        "the model still reads the original refusal text: {refused}"
    );
}
