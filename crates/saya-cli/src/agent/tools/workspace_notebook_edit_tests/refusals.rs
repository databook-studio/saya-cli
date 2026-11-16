//! Every refusal: a stale or missing digest, a cell the notebook does not
//! have, a file that is not a notebook, a source over the bound, and
//! arguments that do not name exactly one cell and one mode. Each leaves the
//! file exactly as it was.

use super::*;
use saya_agent::{ToolError, ToolExecutor};

use super::super::database_tools::WORKSPACE_EDIT_MAX_BYTES;

#[tokio::test]
async fn notebook_edit_refuses_a_stale_digest_and_writes_nothing() {
    let sandbox = Sandbox::new("stale");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");
    // The file moves under the model between the read and the edit.
    sandbox.seed("analysis.ipynb", &NOTEBOOK.replace("print(1)", "print(42)"));

    let error = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "replace",
                "index": 1,
                "source": "print(2)\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("a stale digest must be refused");

    match error {
        ToolError::WorkspaceEditMoved {
            current_digest,
            expected_digest,
            ..
        } => {
            assert_eq!(expected_digest.as_deref(), Some(digest.as_str()));
            assert_ne!(
                current_digest, digest,
                "the refusal names the state the model re-anchors from"
            );
        }
        other => panic!("expected the typed moved-anchor refusal, got: {other}"),
    }
    assert_eq!(
        sandbox.text("analysis.ipynb"),
        NOTEBOOK.replace("print(1)", "print(42)"),
        "a refused edit writes nothing"
    );
}

#[tokio::test]
async fn notebook_edit_requires_a_digest_or_an_earlier_read() {
    let sandbox = Sandbox::new("digest-required");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();

    let error = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "delete",
                "index": 0,
            }),
        )
        .await
        .expect_err("an edit with no observed digest must be refused");

    assert!(
        matches!(error, ToolError::WorkspaceDigestRequired { .. }),
        "expected the read-first refusal, got: {error}"
    );
    assert_eq!(sandbox.text("analysis.ipynb"), NOTEBOOK);
}

#[tokio::test]
async fn notebook_edit_accepts_a_digest_from_an_earlier_read_in_the_same_run() {
    let sandbox = Sandbox::new("read-witness");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();

    tools
        .execute("read", serde_json::json!({"path": "analysis.ipynb"}))
        .await
        .expect("the notebook read must succeed");
    tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "delete",
                "index": 0,
            }),
        )
        .await
        .expect("a read earlier in this run supplies the digest");

    assert_eq!(
        sandbox.json("analysis.ipynb")["cells"]
            .as_array()
            .map(Vec::len),
        Some(2)
    );
}

#[tokio::test]
async fn notebook_edit_refuses_an_index_out_of_range() {
    let sandbox = Sandbox::new("out-of-range");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let error = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "replace",
                "index": 7,
                "source": "x",
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("an index past the last cell must be refused");

    match error {
        ToolError::WorkspaceNotebook { detail, path } => {
            assert_eq!(path, "analysis.ipynb");
            assert!(
                detail.contains('7'),
                "the refusal names the index: {detail}"
            );
            assert!(
                detail.contains('3'),
                "the refusal names the cell count: {detail}"
            );
        }
        other => panic!("expected a typed notebook refusal, got: {other}"),
    }
    assert_eq!(sandbox.text("analysis.ipynb"), NOTEBOOK);
}

#[tokio::test]
async fn notebook_edit_refuses_a_cell_id_no_cell_carries() {
    let sandbox = Sandbox::new("unknown-id");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let error = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "delete",
                "cell_id": "nope",
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("an unknown cell id must be refused");

    assert!(
        matches!(error, ToolError::WorkspaceNotebook { .. }),
        "expected a typed notebook refusal, got: {error}"
    );
    assert_eq!(sandbox.text("analysis.ipynb"), NOTEBOOK);
}

#[tokio::test]
async fn notebook_edit_refuses_a_file_that_is_not_a_notebook() {
    let sandbox = Sandbox::new("not-a-notebook");
    sandbox.seed("notes.ipynb", "{\"hello\": \"world\"}");
    let tools = sandbox.tools();
    let digest = sandbox.digest("notes.ipynb");

    let error = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "notes.ipynb",
                "mode": "delete",
                "index": 0,
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("a file with no cells array must be refused");

    assert!(
        matches!(error, ToolError::WorkspaceNotebook { .. }),
        "expected a typed notebook refusal, got: {error}"
    );
    assert_eq!(sandbox.text("notes.ipynb"), "{\"hello\": \"world\"}");
}

#[tokio::test]
async fn notebook_edit_refuses_source_over_the_byte_bound() {
    let sandbox = Sandbox::new("source-bound");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");
    let source = "x".repeat(WORKSPACE_EDIT_MAX_BYTES + 1);

    let error = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "replace",
                "index": 1,
                "source": source,
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("source over the bound must be refused whole");

    assert!(
        matches!(error, ToolError::WorkspaceEditTooLarge { .. }),
        "expected the whole refusal, got: {error}"
    );
    assert_eq!(sandbox.text("analysis.ipynb"), NOTEBOOK);
}

#[tokio::test]
async fn notebook_edit_refuses_a_missing_mode_and_a_second_cell_selector() {
    let sandbox = Sandbox::new("arguments");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let missing_mode = tools
        .execute(
            "notebook_edit",
            serde_json::json!({"path": "analysis.ipynb", "index": 0, "source": "x"}),
        )
        .await
        .expect_err("a mode is required");
    assert!(
        matches!(missing_mode, ToolError::UnsupportedProperty),
        "got: {missing_mode}"
    );

    let both = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "replace",
                "index": 0,
                "cell_id": "intro",
                "source": "x",
                "expected_digest": digest,
            }),
        )
        .await
        .expect_err("exactly one cell selector is required");
    assert!(
        matches!(both, ToolError::UnsupportedProperty),
        "got: {both}"
    );

    let unknown_mode = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "append",
                "index": 0,
                "source": "x",
            }),
        )
        .await
        .expect_err("an unknown mode is refused with the ones that exist");
    match unknown_mode {
        ToolError::NotebookModeUnsupported { requested } => assert_eq!(requested, "append"),
        other => panic!("expected the typed mode refusal, got: {other}"),
    }
    assert_eq!(sandbox.text("analysis.ipynb"), NOTEBOOK);
}
