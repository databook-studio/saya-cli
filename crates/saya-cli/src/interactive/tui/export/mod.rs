//! Writes a query result to a file as CSV or JSON, chosen by extension.

use saya_types::QueryResult;
use std::path::Path;

mod csv;
mod json;
mod shared;

/// Writes `result` to `path`. Format is chosen by extension: `.csv` or `.json`.
pub(crate) fn write_result(result: &QueryResult, path: &Path) -> Result<usize, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());
    match ext.as_deref() {
        Some("csv") => csv::export_csv(result, path),
        Some("json") => json::export_json(result, path),
        _ => Err("unsupported export format; use a .csv or .json path".into()),
    }
}

// Re-exported so the `#[path]` sibling test module (which resolves `super::`
// to this module) keeps seeing the names the inline tests saw.
#[cfg(test)]
pub(crate) use csv::neutralize_formula;
#[cfg(test)]
pub(crate) use json::disambiguated_columns;

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
