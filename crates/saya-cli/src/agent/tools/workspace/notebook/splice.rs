//! Where an edit writes, and what it writes there.
//!
//! Every arm resolves to one byte range of the file and the bytes that
//! replace it. The ranges are chosen so the comma separating two elements
//! belongs to whichever of them is being rewritten: a replace takes exactly
//! one element's bytes, a delete takes the cell and the comma before it (or,
//! for the first cell, the comma after it), and an insert takes a bare
//! element boundary and puts its own comma on one side. So no byte outside
//! the range moves, and the cells the edit did not name keep their exact
//! bytes — including the notebook's own indentation and key order.

use saya_agent::ToolError;
use serde_json::Value;

use super::args::{Cell, Request};
use super::cells;
use super::scan::Cells;

/// What a refusal says when the file has no notebook structure to edit.
pub(super) const NOT_A_NOTEBOOK: &str = "the file is not a notebook (no `cells` array)";

/// The range this edit replaces, the bytes it puts there, the index of the
/// cell it acted on, and how many cells the notebook holds afterwards.
pub(super) struct Splice {
    pub(super) range: std::ops::Range<usize>,
    pub(super) bytes: Vec<u8>,
    pub(super) cell: usize,
    pub(super) cells: usize,
}

pub(super) fn plan(
    request: &Request,
    notebook: &[u8],
    cells: &Cells,
    path: &str,
) -> Result<Splice, ToolError> {
    let total = cells.elements.len();
    match request {
        Request::Replace {
            cell,
            source,
            cell_type,
            ..
        } => {
            let index = resolve(cells, notebook, cell, path)?;
            let range = cells.elements[index].clone();
            let existing: Value = serde_json::from_slice(&notebook[range.clone()])
                .map_err(|_| not_a_notebook(path))?;
            let rebuilt = cells::rebuild(&existing, source, cell_type.as_deref());
            Ok(Splice {
                range,
                bytes: cells::encode(&rebuilt)?,
                cell: index,
                cells: total,
            })
        }
        Request::Insert {
            index,
            source,
            cell_type,
            ..
        } => {
            let index = usize::try_from(*index)
                .ok()
                .filter(|index| *index <= total)
                .ok_or_else(|| out_of_range(*index, total, path))?;
            let encoded =
                cells::encode(&cells::build(cell_type, source, &cell_ids(cells, notebook)))?;
            let (range, bytes) = if cells.elements.is_empty() {
                (cells.inner.start..cells.inner.start, encoded)
            } else if index == total {
                (
                    cells.elements[total - 1].end..cells.elements[total - 1].end,
                    [b",".as_slice(), &encoded].concat(),
                )
            } else {
                (
                    cells.elements[index].start..cells.elements[index].start,
                    [encoded.as_slice(), b","].concat(),
                )
            };
            Ok(Splice {
                range,
                bytes,
                cell: index,
                cells: total + 1,
            })
        }
        Request::Delete { cell, .. } => {
            let index = resolve(cells, notebook, cell, path)?;
            let range = if total == 1 {
                cells.elements[0].clone()
            } else if index == 0 {
                cells.elements[0].start..cells.elements[1].start
            } else {
                cells.elements[index - 1].end..cells.elements[index].end
            };
            Ok(Splice {
                range,
                bytes: Vec::new(),
                cell: index,
                cells: total - 1,
            })
        }
    }
}

/// The index a `replace` or `delete` names — by position, or by the `id` the
/// cell carries. An id that more than one cell carries is refused rather than
/// resolved to the first: nbformat requires ids to be unique, and a notebook
/// that breaks that must not have the ambiguity decided for it.
fn resolve(cells: &Cells, notebook: &[u8], cell: &Cell, path: &str) -> Result<usize, ToolError> {
    match cell {
        Cell::Index(index) => usize::try_from(*index)
            .ok()
            .filter(|index| *index < cells.elements.len())
            .ok_or_else(|| out_of_range(*index, cells.elements.len(), path)),
        Cell::Id(id) => {
            let mut found = None;
            for (position, range) in cells.elements.iter().enumerate() {
                let parsed: Value = serde_json::from_slice(&notebook[range.clone()])
                    .map_err(|_| not_a_notebook(path))?;
                if parsed.get("id").and_then(Value::as_str) == Some(id.as_str()) {
                    if found.is_some() {
                        return Err(ToolError::WorkspaceNotebook {
                            path: path.to_owned(),
                            detail: format!("more than one cell carries the id {id:?}"),
                        });
                    }
                    found = Some(position);
                }
            }
            found.ok_or_else(|| ToolError::WorkspaceNotebook {
                path: path.to_owned(),
                detail: format!("no cell carries the id {id:?}"),
            })
        }
    }
}

/// The ids the notebook's cells carry, so a new cell's id can be chosen not
/// to repeat one.
fn cell_ids(cells: &Cells, notebook: &[u8]) -> Vec<String> {
    cells
        .elements
        .iter()
        .filter_map(|range| serde_json::from_slice::<Value>(&notebook[range.clone()]).ok())
        .filter_map(|cell| cell.get("id").and_then(Value::as_str).map(str::to_owned))
        .collect()
}

pub(super) fn not_a_notebook(path: &str) -> ToolError {
    ToolError::WorkspaceNotebook {
        path: path.to_owned(),
        detail: NOT_A_NOTEBOOK.to_owned(),
    }
}

/// Names the index the call asked for and how many cells there are, so the
/// model corrects the call from the refusal alone.
fn out_of_range(index: u64, total: usize, path: &str) -> ToolError {
    ToolError::WorkspaceNotebook {
        path: path.to_owned(),
        detail: format!("cell index {index} is out of range (cells: {total})"),
    }
}
