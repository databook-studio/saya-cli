//! File sources (ADR 0007, C1a): staging one local CSV file into a private
//! DuckDB database file plus a preview, without granting access to the
//! source file's parent directory and without the scratch database or a
//! `DatabaseConnector`.
//!
//! The source file is read exactly once through the workspace containment
//! primitives — the parent is opened as a [`Workspace`] solely to open that
//! single final component no-follow with a post-open identity check; nothing
//! else in the parent is listed or opened. The bytes are parsed with the
//! bounded scratch CSV parser (512 columns, 500k rows, 64 KiB fields, UTF-8)
//! and written transactionally to `<dest_dir>/source.duckdb` under the same
//! pinned DuckDB configuration as the scratch database: external access off,
//! autoload off, configuration locked. Stored data is always VARCHAR —
//! inference only labels the preview.

mod csv_stage;
mod infer;
mod parquet_budget;
mod parquet_decode;
mod parquet_preview;
mod parquet_stage;
#[cfg(test)]
mod parquet_tests;
mod preview;
mod read;
#[cfg(test)]
mod tests;

pub use infer::InferredType;
pub use parquet_stage::{ParquetCaps, stage_parquet};
pub use preview::{Preview, PreviewColumn};

use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use thiserror::Error;

use crate::HarnessError;
use crate::scratch::{CsvError, MAX_CSV_ROWS, parse_csv, sanitize_headers};

use self::read::read_source;

/// The staged database file's name inside the destination directory.
pub const STAGED_DB_FILE: &str = "source.duckdb";
/// The metadata table every staged source carries.
pub const RESERVED_METADATA_TABLE: &str = "saya_file_source";
/// The whole-staging wall-clock ceiling: read, parse, and write together.
pub const STAGE_TIMEOUT: Duration = Duration::from_secs(30);

/// The detected source format: routed by the `.parquet` extension or the
/// file's PAR1 magic, with every other byte stream parsed as CSV.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    Csv,
    Parquet,
}

/// Staging options: an explicit delimiter byte (`None` sniffs one from the
/// source's first line) and whether the first row is a header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CsvStageOptions {
    pub delimiter: Option<u8>,
    pub header: bool,
}

/// The result of staging one file: the private DuckDB database file, the
/// single data table (named from the file stem), and a preview. CSV data is
/// stored as VARCHAR; Parquet keeps its native column types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedSource {
    pub db_path: PathBuf,
    pub table: String,
    pub columns: Vec<String>,
    pub rows: usize,
    pub sha256: String,
    pub bytes: u64,
    pub preview: Preview,
    pub format: SourceFormat,
}

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

/// Stages one local file into a private DuckDB database file at
/// `<dest_dir>/source.duckdb` (dir 0700, file 0600), routing by the
/// `.parquet` extension or PAR1 magic: Parquet through the bounded staging
/// decode, everything else through the bounded CSV parser. The source file
/// is read exactly once; the write is transactional; any failure leaves no
/// staged file behind.
pub fn stage_source(
    source: &Path,
    dest_dir: &Path,
    options: CsvStageOptions,
) -> Result<StagedSource, StageError> {
    let read = read_source(source)?;
    if parquet_stage::looks_like_parquet(&read) {
        parquet_stage::stage_read(read, dest_dir, ParquetCaps::default())
    } else {
        stage_csv_read(read, dest_dir, options)
    }
}

/// Stages one local CSV file into a private DuckDB database file at
/// `<dest_dir>/source.duckdb` (dir 0700, file 0600), returning the staged
/// source's receipt and preview. The source file is read exactly once; the
/// write is transactional; any failure leaves no staged file behind.
pub fn stage_csv(
    source: &Path,
    dest_dir: &Path,
    options: CsvStageOptions,
) -> Result<StagedSource, StageError> {
    let read = read_source(source)?;
    stage_csv_read(read, dest_dir, options)
}

/// The CSV pipeline from an already-contained read, shared by `stage_csv`
/// and the CSV arm of `stage_source`.
fn stage_csv_read(
    read: read::SourceRead,
    dest_dir: &Path,
    options: CsvStageOptions,
) -> Result<StagedSource, StageError> {
    let deadline = Instant::now() + STAGE_TIMEOUT;
    let delimiter = preview::resolve_delimiter(options.delimiter, &read.bytes);
    let parsed = parse_csv(&read.bytes, delimiter)?;
    if parsed.is_empty() {
        return Err(StageError::Empty);
    }
    let (columns, data): (Vec<String>, &[Vec<String>]) = if options.header {
        (sanitize_headers(&parsed[0]), &parsed[1..])
    } else {
        let count = parsed[0].len();
        (
            (1..=count).map(|index| format!("column_{index}")).collect(),
            &parsed[..],
        )
    };
    if data.len() > MAX_CSV_ROWS {
        return Err(CsvError::TooManyRows { max: MAX_CSV_ROWS }.into());
    }
    for (index, row) in data.iter().enumerate() {
        if row.len() != columns.len() {
            return Err(StageError::RaggedRow {
                row: index + 1,
                found: row.len(),
                expected: columns.len(),
            });
        }
    }
    let table = table_name(&read.stem);
    if table == RESERVED_METADATA_TABLE {
        return Err(StageError::ReservedTableName { name: table });
    }
    prepare_dest_dir(dest_dir)?;
    csv_stage::write_staged(dest_dir, &table, &columns, data, &read, deadline)?;
    let preview = preview::build(delimiter, options.header, &columns, data);
    Ok(StagedSource {
        db_path: dest_dir.join(STAGED_DB_FILE),
        table,
        columns,
        rows: data.len(),
        sha256: read.sha256,
        bytes: read.size,
        preview,
        format: SourceFormat::Csv,
    })
}

/// Ensures the destination directory exists at 0700, is a real directory,
/// and is not a symlink.
fn prepare_dest_dir(dest_dir: &Path) -> Result<(), StageError> {
    match fs::create_dir(dest_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(StageError::Destination(format!(
                "create {}: {error}",
                dest_dir.display()
            )));
        }
    }
    let metadata = fs::symlink_metadata(dest_dir).map_err(|error| {
        StageError::Destination(format!("stat {}: {error}", dest_dir.display()))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(StageError::Destination(format!(
            "{} is not a real directory",
            dest_dir.display()
        )));
    }
    #[cfg(unix)]
    fs::set_permissions(dest_dir, fs::Permissions::from_mode(0o700)).map_err(|error| {
        StageError::Destination(format!("restrict {} to 0700: {error}", dest_dir.display()))
    })?;
    Ok(())
}

/// The staged table's name: the file stem lowercased to `[a-z0-9_]{1,63}`,
/// a leading digit gaining the `t_` prefix.
fn table_name(stem: &str) -> String {
    let mut name: String = stem
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if name
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_digit())
    {
        name.insert_str(0, "t_");
    }
    if name.is_empty() {
        return "source".to_owned();
    }
    name.truncate(63);
    name
}
