//! Tests for the notebook arm of `read`: a `.ipynb` whose bytes parse as an
//! nbformat notebook is served as its cells — index, type, source and outputs
//! as text — never as a text view of its JSON. A file named `.ipynb` that is
//! not a notebook keeps the text path, and a line range always reads the file's
//! own raw lines. The image outputs have their own file: [`super::notebook_images`].

use super::*;
use crate::agent::tools::workspace::notebook::NOTEBOOK_MAX_CELLS;
use base64::Engine as _;

/// One notebook's bytes, with the layout a real `.ipynb` has: a top-level
/// object whose `cells` array sits beside `metadata` and `nbformat`.
pub(super) fn notebook(cells: serde_json::Value) -> Vec<u8> {
    serde_json::to_vec_pretty(&serde_json::json!({
        "cells": cells,
        "metadata": {"language_info": {"name": "python"}},
        "nbformat": 4,
        "nbformat_minor": 5,
    }))
    .expect("the notebook fixture must serialize")
}

/// Writes `cells` as a notebook at `path`, answering the bytes' length.
pub(super) fn seed(sandbox: &Sandbox, path: &str, cells: serde_json::Value) -> u64 {
    let bytes = notebook(cells);
    sandbox
        .ws
        .write(path, &bytes)
        .expect("contained write must succeed");
    bytes.len() as u64
}

/// The PNG signature, base64 — a real image's first bytes, which is all the
/// media type is read from.
pub(super) const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";

pub(super) fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// One code cell whose only output is an image of `data`, declared as `mime`.
pub(super) fn image_cell(id: &str, mime: &str, data: &str) -> serde_json::Value {
    serde_json::json!({
        "cell_type": "code",
        "id": id,
        "metadata": {},
        "outputs": [{
            "output_type": "display_data",
            "metadata": {},
            "data": {mime: data, "text/plain": ["<Figure>"]},
        }],
        "source": ["plot()"],
    })
}

#[tokio::test]
async fn read_of_a_notebook_shows_each_cells_index_type_source_and_outputs() {
    let sandbox = Sandbox::new("notebook-cells");
    let size = seed(
        &sandbox,
        "analysis.ipynb",
        serde_json::json!([
            {"cell_type": "markdown", "id": "intro", "metadata": {},
             "source": ["# Title\n", "hello"]},
            {"cell_type": "code", "execution_count": 1, "id": "code-1", "metadata": {},
             "outputs": [{"name": "stdout", "output_type": "stream", "text": ["1\n"]}],
             "source": ["print(1)"]},
        ]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute("read", serde_json::json!({"path": "analysis.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(content.contains("cell 0 [markdown] id=intro"), "{content}");
    assert!(content.contains("# Title"), "{content}");
    assert!(content.contains("hello"), "{content}");
    assert!(content.contains("cell 1 [code] id=code-1"), "{content}");
    assert!(content.contains("print(1)"), "{content}");
    assert!(content.contains("output 0 [stream]"), "{content}");
    assert!(content.contains("1\n"), "{content}");
    assert_eq!(result["cells"].as_u64(), Some(2));
    assert_eq!(result["cells_shown"].as_u64(), Some(2));
    assert!(
        result.get("truncated").is_none(),
        "a notebook served whole carries no `truncated`: {result}"
    );
    assert_eq!(result["size"].as_u64(), Some(size));
    assert_eq!(
        result["digest"].as_str().map(str::len),
        Some(64),
        "the notebook read still names the whole-file digest an edit states: {result}"
    );
}

#[tokio::test]
async fn read_of_a_notebook_counts_the_cells_it_did_not_show() {
    let sandbox = Sandbox::new("notebook-cell-cap");
    let total = NOTEBOOK_MAX_CELLS + 5;
    let cells: Vec<serde_json::Value> = (0..total)
        .map(|index| {
            serde_json::json!({
                "cell_type": "code",
                "id": format!("cell-{index}"),
                "metadata": {},
                "outputs": [],
                "source": ["x"],
            })
        })
        .collect();
    seed(&sandbox, "many.ipynb", serde_json::Value::Array(cells));
    let tools = sandbox.tools();

    let result = tools
        .execute("read", serde_json::json!({"path": "many.ipynb"}))
        .await
        .expect("a contained notebook read must succeed");

    assert_eq!(result["cells"].as_u64(), Some(total as u64));
    assert_eq!(
        result["cells_shown"].as_u64(),
        Some(NOTEBOOK_MAX_CELLS as u64)
    );
    assert_eq!(result["truncated"], serde_json::Value::Bool(true));
    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(
        content.contains("[5 more cell(s) not shown]"),
        "the rest are marked, not silently dropped: {content}"
    );
}

#[tokio::test]
async fn read_of_a_line_range_of_a_notebook_serves_the_files_own_lines() {
    let sandbox = Sandbox::new("notebook-range");
    seed(
        &sandbox,
        "analysis.ipynb",
        serde_json::json!([
            {"cell_type": "markdown", "id": "intro", "metadata": {}, "source": ["# Title"]},
        ]),
    );
    let tools = sandbox.tools();

    let result = tools
        .execute(
            "read",
            serde_json::json!({"path": "analysis.ipynb", "start_line": 1, "line_count": 3}),
        )
        .await
        .expect("a ranged read of a notebook must succeed");

    let content = result["content"]
        .as_str()
        .expect("content must be a string");
    assert!(
        content.contains("1→{"),
        "a line range reads the file's own raw lines: {content}"
    );
    assert!(
        !content.contains("cell 0 [markdown]"),
        "a line range does not serve the cell view: {content}"
    );
}

#[tokio::test]
async fn read_of_a_json_file_named_ipynb_that_is_not_a_notebook_keeps_the_text_path() {
    let sandbox = Sandbox::new("notebook-not-a-notebook");
    sandbox
        .ws
        .write("notes.ipynb", b"{\"hello\": \"world\"}")
        .expect("contained write must succeed");
    let tools = sandbox.tools();

    let result = tools
        .execute("read", serde_json::json!({"path": "notes.ipynb"}))
        .await
        .expect("a contained read must succeed");

    assert_eq!(result["content"], "     1→{\"hello\": \"world\"}");
    assert!(
        result.get("cells").is_none(),
        "a file that is not a notebook is served as the text it is: {result}"
    );
}
