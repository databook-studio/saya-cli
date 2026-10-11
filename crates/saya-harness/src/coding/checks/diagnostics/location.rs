use std::path::Path;

use serde_json::Value;

use crate::coding::snapshot::SnapshotPath;

use super::types::{DiagnosticError, DiagnosticLocation};

const MAX_LOCATION_PATH_BYTES: usize = 512;
const MAX_LINE: u64 = 10_000_000;
const MAX_COLUMN: u64 = 1_000_000;
const MAX_SPANS: usize = 128;
const MAX_PRIMARY_LOCATIONS: usize = 8;

pub(super) fn primary_locations(
    spans: &Value,
    candidate_root: &Path,
    line: usize,
) -> Result<Vec<DiagnosticLocation>, DiagnosticError> {
    let spans = spans
        .as_array()
        .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
    if spans.len() > MAX_SPANS {
        return Err(DiagnosticError::InvalidDiagnostic { line });
    }

    let mut locations = Vec::new();
    for span in spans {
        let span = span
            .as_object()
            .ok_or(DiagnosticError::InvalidDiagnostic { line })?;
        let file = span
            .get("file_name")
            .and_then(Value::as_str)
            .ok_or(DiagnosticError::InvalidLocation { line })?;
        if file.len() > MAX_LOCATION_PATH_BYTES || has_controls(file) {
            return Err(DiagnosticError::InvalidLocation { line });
        }
        let line_start = bounded(span.get("line_start"), 1, MAX_LINE, line)?;
        let line_end = bounded(span.get("line_end"), 1, MAX_LINE, line)?;
        let column_start = bounded(span.get("column_start"), 1, MAX_COLUMN, line)?;
        let column_end = bounded(span.get("column_end"), 1, MAX_COLUMN, line)?;
        if (line_start, column_start) > (line_end, column_end) {
            return Err(DiagnosticError::InvalidLocation { line });
        }
        let primary = span
            .get("is_primary")
            .and_then(Value::as_bool)
            .ok_or(DiagnosticError::InvalidLocation { line })?;
        if primary {
            let path = candidate_relative_path(file, candidate_root)
                .ok_or(DiagnosticError::InvalidLocation { line })?;
            locations.push(DiagnosticLocation::new(
                path,
                line_start,
                line_end,
                column_start,
                column_end,
            ));
            if locations.len() > MAX_PRIMARY_LOCATIONS {
                return Err(DiagnosticError::InvalidDiagnostic { line });
            }
        }
    }
    if !spans.is_empty() && locations.is_empty() {
        return Err(DiagnosticError::InvalidLocation { line });
    }
    locations.sort();
    locations.dedup();
    Ok(locations)
}

fn bounded(value: Option<&Value>, min: u64, max: u64, line: usize) -> Result<u32, DiagnosticError> {
    let value = value
        .and_then(Value::as_u64)
        .filter(|value| (min..=max).contains(value))
        .ok_or(DiagnosticError::InvalidLocation { line })?;
    Ok(value as u32)
}

fn candidate_relative_path(file: &str, candidate_root: &Path) -> Option<String> {
    let root = candidate_root.to_str()?;
    if !candidate_root.is_absolute() || has_controls(root) || root.contains('\\') {
        return None;
    }
    let relative = if Path::new(file).is_absolute() {
        let prefix = root.trim_end_matches('/');
        let suffix = if prefix.is_empty() {
            file.strip_prefix('/')?
        } else {
            file.strip_prefix(prefix)?.strip_prefix('/')?
        };
        if suffix.is_empty() {
            return None;
        }
        let joined = if prefix.is_empty() {
            format!("/{suffix}")
        } else {
            format!("{prefix}/{suffix}")
        };
        if joined != file {
            return None;
        }
        suffix
    } else {
        file
    };
    if relative.contains('\\')
        || relative
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return None;
    }
    SnapshotPath::parse(relative.to_owned())
        .ok()
        .map(|path| path.as_str().to_owned())
}

fn has_controls(value: &str) -> bool {
    value.chars().any(char::is_control)
}
