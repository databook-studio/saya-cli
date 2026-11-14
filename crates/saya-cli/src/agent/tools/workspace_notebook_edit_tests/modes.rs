//! The three modes, each round-tripping to a notebook that is still valid
//! nbformat JSON with the cells it did not name byte-for-byte what they were.

use super::*;
use saya_agent::ToolExecutor;

#[tokio::test]
async fn notebook_edit_replaces_one_cell_and_leaves_the_others_byte_identical() {
    let sandbox = Sandbox::new("replace");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let result = tools
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
        .expect("replacing one cell must succeed");

    assert_eq!(result["path"], "analysis.ipynb");
    assert_eq!(result["mode"], "replace");
    assert_eq!(result["cell"].as_u64(), Some(1));
    assert_eq!(result["cells"].as_u64(), Some(3));
    assert_eq!(
        result["digest"].as_str(),
        Some(sandbox.digest("analysis.ipynb").as_str()),
        "the result names the state the next call's precondition states"
    );
    let notebook = sandbox.json("analysis.ipynb");
    assert_eq!(notebook["cells"].as_array().map(Vec::len), Some(3));
    assert_eq!(cell_source(&notebook["cells"][1]), "print(2)\n");
    assert_eq!(notebook["cells"][1]["cell_type"], "code");
    assert_eq!(
        notebook["cells"][1]["outputs"][0]["text"][0], "1\n",
        "a replace keeps the cell's own outputs"
    );
    assert_eq!(notebook["nbformat"], 4);
    assert_eq!(notebook["metadata"]["language_info"]["name"], "python");
    assert!(
        sandbox.text("analysis.ipynb").contains(KEEP),
        "an untouched cell keeps its exact bytes: {}",
        sandbox.text("analysis.ipynb")
    );
}

#[tokio::test]
async fn notebook_edit_inserts_a_cell_at_the_index() {
    let sandbox = Sandbox::new("insert");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let result = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "insert",
                "index": 1,
                "cell_type": "markdown",
                "source": "# Inserted\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("inserting one cell must succeed");

    assert_eq!(result["cell"].as_u64(), Some(1));
    assert_eq!(result["cells"].as_u64(), Some(4));
    let notebook = sandbox.json("analysis.ipynb");
    assert_eq!(notebook["cells"].as_array().map(Vec::len), Some(4));
    assert_eq!(notebook["cells"][1]["cell_type"], "markdown");
    assert_eq!(cell_source(&notebook["cells"][1]), "# Inserted\n");
    assert_eq!(
        notebook["cells"][2]["id"], "code-1",
        "the cell that was at the index moved down, unchanged"
    );
    assert!(sandbox.text("analysis.ipynb").contains(KEEP));
}

#[tokio::test]
async fn notebook_edit_inserts_at_the_end_of_the_cells() {
    let sandbox = Sandbox::new("insert-end");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "insert",
                "index": 3,
                "cell_type": "code",
                "source": "print(3)\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("inserting at the end must succeed");

    let notebook = sandbox.json("analysis.ipynb");
    assert_eq!(notebook["cells"].as_array().map(Vec::len), Some(4));
    assert_eq!(cell_source(&notebook["cells"][3]), "print(3)\n");
    assert_eq!(notebook["cells"][3]["outputs"], serde_json::json!([]));
    assert!(
        notebook["cells"][3]["id"].is_string(),
        "a new cell carries an id when the notebook's cells do: {notebook}"
    );
}

#[tokio::test]
async fn notebook_edit_deletes_the_cell_at_the_index() {
    let sandbox = Sandbox::new("delete");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let result = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "delete",
                "index": 0,
                "expected_digest": digest,
            }),
        )
        .await
        .expect("deleting one cell must succeed");

    assert_eq!(result["mode"], "delete");
    assert_eq!(result["cells"].as_u64(), Some(2));
    let notebook = sandbox.json("analysis.ipynb");
    assert_eq!(notebook["cells"].as_array().map(Vec::len), Some(2));
    assert_eq!(notebook["cells"][0]["id"], "code-1");
    assert_eq!(notebook["cells"][1]["id"], "keep");
    assert!(sandbox.text("analysis.ipynb").contains(KEEP));
}

#[tokio::test]
async fn notebook_edit_resolves_a_cell_by_id() {
    let sandbox = Sandbox::new("by-id");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    let result = tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "replace",
                "cell_id": "code-1",
                "source": "print(9)\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("a cell id must resolve");

    assert_eq!(result["cell"].as_u64(), Some(1));
    let notebook = sandbox.json("analysis.ipynb");
    assert_eq!(cell_source(&notebook["cells"][1]), "print(9)\n");
    assert!(sandbox.text("analysis.ipynb").contains(KEEP));
}

#[tokio::test]
async fn notebook_edit_changes_a_cells_type_when_asked() {
    let sandbox = Sandbox::new("cell-type");
    sandbox.seed("analysis.ipynb", NOTEBOOK);
    let tools = sandbox.tools();
    let digest = sandbox.digest("analysis.ipynb");

    tools
        .execute(
            "notebook_edit",
            serde_json::json!({
                "path": "analysis.ipynb",
                "mode": "replace",
                "index": 1,
                "cell_type": "markdown",
                "source": "no longer code\n",
                "expected_digest": digest,
            }),
        )
        .await
        .expect("changing the cell type must succeed");

    let notebook = sandbox.json("analysis.ipynb");
    assert_eq!(notebook["cells"][1]["cell_type"], "markdown");
    assert_eq!(cell_source(&notebook["cells"][1]), "no longer code\n");
    assert!(
        notebook["cells"][1].get("outputs").is_none(),
        "a cell that is no longer code carries no outputs: {}",
        notebook["cells"][1]
    );
}
