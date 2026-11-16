//! Building the cell an edit writes.
//!
//! A cell is small and its shape is nbformat's, so it is built as a
//! [`Value`] and encoded once — unlike the notebook around it, which is
//! spliced as bytes (see [`super::scan`]). Two forms: [`rebuild`] keeps what
//! the cell at that index already carried that its type still takes, and
//! [`build`] is a cell that did not exist.

use saya_agent::ToolError;
use serde_json::Value;

/// The cell types `notebook_edit` writes. nbformat defines these two; the
/// scanner and the view pass anything else through untouched.
pub(super) const CELL_TYPES: &[&str] = &["code", "markdown"];

/// The cell written in place of `cell`: its type (`cell_type` when the call
/// names one, otherwise its own), its new source, and — carried across
/// unchanged — its `id`, its `metadata`, and, while it is still a code cell,
/// its `outputs` and `execution_count`. Changing a code cell to markdown
/// drops those two, because nbformat has no place for them on a markdown
/// cell; nothing else about the cell is touched.
pub(super) fn rebuild(cell: &Value, source: &str, cell_type: Option<&str>) -> Value {
    let kind = cell_type.unwrap_or_else(|| {
        cell.get("cell_type")
            .and_then(Value::as_str)
            .unwrap_or("code")
    });
    let mut rebuilt = serde_json::Map::new();
    rebuilt.insert("cell_type".into(), Value::String(kind.to_owned()));
    if let Some(id) = cell.get("id") {
        rebuilt.insert("id".into(), id.clone());
    }
    if kind == "code" {
        rebuilt.insert(
            "execution_count".into(),
            cell.get("execution_count").cloned().unwrap_or(Value::Null),
        );
        rebuilt.insert(
            "outputs".into(),
            cell.get("outputs").cloned().unwrap_or_else(empty_array),
        );
    }
    rebuilt.insert(
        "metadata".into(),
        cell.get("metadata").cloned().unwrap_or_else(empty_object),
    );
    rebuilt.insert("source".into(), source_value(source));
    Value::Object(rebuilt)
}

/// A cell that did not exist before: `cell_type`, `source`, empty metadata,
/// and, for a code cell, no outputs and no execution count.
///
/// It carries an `id` only when the notebook's own cells do — an id is
/// required from nbformat 4.5 on and forbidden before it, so one insert must
/// not half-upgrade a notebook that predates it.
pub(super) fn build(cell_type: &str, source: &str, ids: &[String]) -> Value {
    let mut cell = serde_json::Map::new();
    cell.insert("cell_type".into(), Value::String(cell_type.to_owned()));
    if !ids.is_empty() {
        cell.insert("id".into(), Value::String(fresh_id(ids)));
    }
    if cell_type == "code" {
        cell.insert("execution_count".into(), Value::Null);
        cell.insert("outputs".into(), empty_array());
    }
    cell.insert("metadata".into(), empty_object());
    cell.insert("source".into(), source_value(source));
    Value::Object(cell)
}

pub(super) fn encode(cell: &Value) -> Result<Vec<u8>, ToolError> {
    serde_json::to_vec(cell).map_err(|error| {
        ToolError::WorkspaceEdit(format!("the notebook cell could not be written: {error}"))
    })
}

/// The first `cell-N` no cell already carries. Unique by construction, and
/// well inside nbformat's own bound on an id's length.
fn fresh_id(ids: &[String]) -> String {
    let mut number = 1usize;
    loop {
        let candidate = format!("cell-{number}");
        if !ids.iter().any(|id| id == &candidate) {
            return candidate;
        }
        number += 1;
    }
}

/// nbformat's own form for a cell's text: the lines, each keeping its
/// newline. A source that does not end in a newline is still one line, so
/// this never invents a trailing one.
fn source_value(source: &str) -> Value {
    Value::Array(
        source
            .split_inclusive('\n')
            .map(|line| Value::String(line.to_owned()))
            .collect(),
    )
}

fn empty_array() -> Value {
    Value::Array(Vec::new())
}

fn empty_object() -> Value {
    Value::Object(serde_json::Map::new())
}
