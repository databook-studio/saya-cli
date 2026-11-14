//! `notebook_edit`'s argument typing: which of the three modes was asked for,
//! which cell it names, and what it writes. Refusals here are argument
//! refusals — nothing has been read yet — and each names the shape the tool
//! actually takes, so the model corrects the call in one turn.

use saya_agent::ToolError;
use serde_json::Value;

use super::cells::CELL_TYPES;

/// The keys `notebook_edit` takes.
const ALLOWED: &[&str] = &[
    "path",
    "mode",
    "index",
    "cell_id",
    "source",
    "cell_type",
    "expected_digest",
];

/// Which cell a `replace` or `delete` acts on.
#[derive(Debug)]
pub(super) enum Cell {
    /// The 0-based position in the `cells` array.
    Index(u64),
    /// The cell's own `id`, which nbformat 4.5 requires to be unique.
    Id(String),
}

/// One parsed `notebook_edit` call.
#[derive(Debug)]
pub(super) enum Request {
    Replace {
        path: String,
        cell: Cell,
        source: String,
        /// The type to write; `None` keeps the cell's own.
        cell_type: Option<String>,
        expected_digest: Option<String>,
    },
    Insert {
        path: String,
        index: u64,
        source: String,
        cell_type: String,
        expected_digest: Option<String>,
    },
    Delete {
        path: String,
        cell: Cell,
        expected_digest: Option<String>,
    },
}

impl Request {
    pub(super) fn path(&self) -> &str {
        match self {
            Request::Replace { path, .. }
            | Request::Insert { path, .. }
            | Request::Delete { path, .. } => path,
        }
    }

    pub(super) fn expected_digest(&self) -> Option<&str> {
        match self {
            Request::Replace {
                expected_digest, ..
            }
            | Request::Insert {
                expected_digest, ..
            }
            | Request::Delete {
                expected_digest, ..
            } => expected_digest.as_deref(),
        }
    }

    /// The mode's own name, as the result and the refusal spell it.
    pub(super) fn mode(&self) -> &'static str {
        match self {
            Request::Replace { .. } => "replace",
            Request::Insert { .. } => "insert",
            Request::Delete { .. } => "delete",
        }
    }
}

pub(super) fn parse_arguments(arguments: &Value) -> Result<Request, ToolError> {
    let object = arguments.as_object().ok_or(ToolError::ArgumentsNotObject)?;
    if object.keys().any(|key| !ALLOWED.contains(&key.as_str())) {
        return Err(ToolError::UnsupportedProperty);
    }
    let path = object
        .get("path")
        .and_then(Value::as_str)
        .ok_or(ToolError::PathNotString)?
        .to_owned();
    let mode = object
        .get("mode")
        .and_then(Value::as_str)
        .ok_or(ToolError::UnsupportedProperty)?;
    let expected_digest = match object.get("expected_digest") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .ok_or(ToolError::ExpectedDigestNotString)?
                .to_owned(),
        ),
    };
    let index = match object.get("index") {
        None | Some(Value::Null) => None,
        Some(value) => Some(value.as_u64().ok_or(ToolError::UnsupportedProperty)?),
    };
    let cell_id = match object.get("cell_id") {
        None | Some(Value::Null) => None,
        Some(value) => Some(
            value
                .as_str()
                .filter(|id| !id.is_empty())
                .ok_or(ToolError::UnsupportedProperty)?
                .to_owned(),
        ),
    };
    let source = object
        .get("source")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let cell_type = match object.get("cell_type") {
        None | Some(Value::Null) => None,
        Some(value) => {
            let name = value.as_str().ok_or(ToolError::UnsupportedProperty)?;
            if !CELL_TYPES.contains(&name) {
                return Err(ToolError::UnsupportedProperty);
            }
            Some(name.to_owned())
        }
    };
    match mode {
        "replace" => Ok(Request::Replace {
            path,
            cell: one_cell(index, cell_id)?,
            source: source.ok_or(ToolError::UnsupportedProperty)?,
            cell_type,
            expected_digest,
        }),
        "insert" => {
            // An insert names a position, never an existing cell: there is no
            // cell to resolve an id against.
            if cell_id.is_some() {
                return Err(ToolError::UnsupportedProperty);
            }
            Ok(Request::Insert {
                path,
                index: index.ok_or(ToolError::UnsupportedProperty)?,
                source: source.ok_or(ToolError::UnsupportedProperty)?,
                cell_type: cell_type.ok_or(ToolError::UnsupportedProperty)?,
                expected_digest,
            })
        }
        "delete" => {
            if source.is_some() || cell_type.is_some() {
                return Err(ToolError::UnsupportedProperty);
            }
            Ok(Request::Delete {
                path,
                cell: one_cell(index, cell_id)?,
                expected_digest,
            })
        }
        other => Err(ToolError::NotebookModeUnsupported {
            requested: other.chars().take(32).collect(),
        }),
    }
}

/// Exactly one of `index` and `cell_id` names the cell — never neither, never
/// both, so the tool cannot act on a cell the model did not mean.
fn one_cell(index: Option<u64>, cell_id: Option<String>) -> Result<Cell, ToolError> {
    match (index, cell_id) {
        (Some(index), None) => Ok(Cell::Index(index)),
        (None, Some(id)) => Ok(Cell::Id(id)),
        _ => Err(ToolError::UnsupportedProperty),
    }
}
