//! Writes a query result to a file as CSV or JSON, chosen by extension.
//!
//! Every write is two phases: encode into memory under a hard ceiling
//! (checked while encoding, so an oversize result stops early), then
//! publish atomically — private temp file in the destination's directory,
//! fsync, rename — behind an explicit overwrite decision. A failed or
//! refused export leaves any existing destination byte-for-byte unchanged
//! and no temp file behind.

use saya_types::QueryResult;
use std::path::Path;

mod atomic;
mod csv;
mod json;
mod shared;

/// The hard ceiling on one encoded export, checked while encoding.
pub(crate) const MAX_EXPORT_BYTES: usize = 32 * 1024 * 1024;

/// Writes `result` to `path`, refusing an existing destination unless
/// `overwrite`. Format is chosen by extension: `.csv` or `.json`.
pub(crate) fn write_result_overwrite(
    result: &QueryResult,
    path: &Path,
    overwrite: bool,
) -> Result<usize, String> {
    write_within(result, path, overwrite, MAX_EXPORT_BYTES)
}

/// Test seam: a smaller ceiling exercises the while-encoding stop without
/// allocating a 32 MiB result.
#[cfg(test)]
pub(crate) fn write_result_with_ceiling(
    result: &QueryResult,
    path: &Path,
    overwrite: bool,
    ceiling: usize,
) -> Result<usize, String> {
    write_within(result, path, overwrite, ceiling)
}

fn write_within(
    result: &QueryResult,
    path: &Path,
    overwrite: bool,
    ceiling: usize,
) -> Result<usize, String> {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_lowercase());
    // Fail fast on the destination before encoding anything.
    atomic::guard(path, overwrite)?;
    let bytes = match ext.as_deref() {
        Some("csv") => csv::encode_csv(result, ceiling)?,
        Some("json") => json::encode_json(result, ceiling)?,
        _ => return Err("unsupported export format; use a .csv or .json path".into()),
    };
    atomic::publish(path, &bytes, overwrite)?;
    Ok(result.rows.len())
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
