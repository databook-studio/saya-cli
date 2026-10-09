//! The file source's staging refusals.

use std::{io, path::PathBuf};

use thiserror::Error;

use crate::HarnessError;
use crate::scratch::CsvError;

/// Staging refusals. None carry source field values.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StageError {
    /// The contained source read refused: symlink, non-regular file, bounds,
    /// invalid path, or I/O failure.
    #[error("file source refused: {0}")]
    Source(#[from] HarnessError),
    /// The bounded CSV parser refused the source.
    #[error("CSV source refused: {0}")]
    Csv(#[from] CsvError),
    /// A data row's field count differs from the column count. `row` is the
    /// 1-based data-row index.
    #[error("CSV data row {row} has {found} fields; expected {expected}")]
    RaggedRow {
        row: usize,
        found: usize,
        expected: usize,
    },
    #[error("CSV file has no rows")]
    Empty,
    /// The file stem sanitises to the reserved metadata table's name.
    #[error("file stem sanitises to the reserved metadata table name {name:?}; staging is refused")]
    ReservedTableName { name: String },
    #[error("staging destination unusable: {0}")]
    Destination(String),
    /// The staged file's mode could not be restricted to 0600.
    #[error("staged file could not be restricted to 0600: {path}")]
    FileMode {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("staging timed out")]
    Timeout,
    #[error("staging failed at the database layer")]
    Database,
    /// The Parquet decode hit a DuckDB error outside the typed refusals —
    /// a corrupt or non-Parquet file, an interrupted decode, an I/O failure.
    #[error("Parquet staging failed: {0}")]
    ParquetFailed(String),
    /// Parquet metadata reports more rows than the decode cap, before any
    /// row is decoded.
    #[error("Parquet file has {rows} rows; at most {max} can be staged")]
    ParquetTooManyRows { rows: u64, max: u64 },
    /// Parquet metadata reports more columns than the cap.
    #[error("Parquet file has {columns} columns; at most {max} can be staged")]
    ParquetTooManyColumns { columns: usize, max: usize },
    /// The staged decode produced more accounted cell bytes than the
    /// decoded-byte budget allows.
    #[error(
        "Parquet file decodes to {bytes} bytes of cell data; at most {max} bytes can be staged"
    )]
    ParquetTooManyDecodedBytes { bytes: u64, max: u64 },
    /// A Parquet column's type is nested (STRUCT/LIST/MAP/UNION/ARRAY).
    #[error("Parquet column {column:?} has nested type {kind}; only flat columns can be staged")]
    ParquetNestedColumn { column: String, kind: String },
    #[error("Parquet file has no columns")]
    ParquetNoColumns,
}
