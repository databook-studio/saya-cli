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

mod csv_pipeline;
mod csv_stage;
mod error;
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

pub use error::StageError;
pub use infer::InferredType;
pub use parquet_stage::{ParquetCaps, stage_parquet};
pub use preview::{Preview, PreviewColumn};

use std::{
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

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
        csv_pipeline::stage_csv_read(read, dest_dir, options)
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
    csv_pipeline::stage_csv_read(read, dest_dir, options)
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
