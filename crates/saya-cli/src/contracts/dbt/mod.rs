//! The bounded dbt manifest parser (B3a, ADR 0006): turns a selected slice of
//! a dbt `manifest.json` into portable context items without executing or
//! following anything.
//!
//! The manifest is untrusted input. [`parse_dbt_manifest`] reads at most
//! [`MAX_MANIFEST_BYTES`] bytes, refuses schema versions outside v10-v12 by
//! name, and maps only model and source nodes — descriptions, column
//! descriptions, and `relationships` tests — through the existing validating
//! constructors in `saya-types`. Anything a constructor refuses is skipped and
//! reported, never loosened. No dbt process runs here, no macro or compiled
//! SQL is read into the output, and nothing is written to the store: the
//! `import-dbt` command owns that step.

mod manifest;
mod map;
mod relationships;
mod resolve;
mod select;

#[cfg(test)]
mod tests;

pub(crate) use manifest::DbtVersion;

use std::path::Path;

use saya_types::ContextItem;

/// A manifest file is read only up to this many bytes; anything larger is
/// refused before a byte is parsed.
pub(crate) const MAX_MANIFEST_BYTES: usize = 32 * 1024 * 1024;
/// At most this many model/source nodes may pass selection; a larger import
/// is refused rather than truncated.
pub(crate) const MAX_SELECTED_NODES: usize = 5_000;

/// Why the manifest reader refused an import outright. Item-level failures
/// are not errors — they are reported per item in [`DbtImport::skipped`].
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DbtManifestError {
    #[error("dbt manifest is {0} bytes, over the {MAX_MANIFEST_BYTES}-byte limit")]
    Oversize(usize),
    #[error("dbt manifest is not valid JSON")]
    Malformed,
    #[error("not a dbt manifest: metadata.dbt_schema_version is missing")]
    NotAManifest,
    #[error("unsupported dbt manifest schema version {0} (supported: v10, v11, v12)")]
    UnsupportedVersion(String),
    #[error("{0} model/source nodes selected, over the {MAX_SELECTED_NODES}-node limit")]
    TooManyNodes(usize),
    #[error("could not read the dbt manifest: {0}")]
    Io(#[from] std::io::Error),
}

/// What one manifest import produced: the portable context items it mapped,
/// every item skipped with a stable reason code, and the manifest schema
/// version they came from.
#[derive(Debug)]
pub struct DbtImport {
    pub items: Vec<ContextItem>,
    /// `(unique_id, reason)` for every refused item, in the order the mapping
    /// encountered them. Reasons are stable short codes; see `map`.
    pub skipped: Vec<(String, &'static str)>,
    pub version: DbtVersion,
}

/// Parses a dbt manifest at `path` into context items, selecting model and
/// source nodes whose name matches any of the `select` globs (`*` and `?`;
/// an empty slice selects everything).
pub(crate) fn parse_dbt_manifest(
    path: &Path,
    select: &[String],
) -> Result<DbtImport, DbtManifestError> {
    let selected = select::read_and_select(path, select)?;
    Ok(map::map(selected))
}
