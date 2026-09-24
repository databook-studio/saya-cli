//! Transactional CSV insertion into the already-open scratch database.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use duckdb::params_from_iter;
use serde::Serialize;
use thiserror::Error;

use super::csv::{CsvError, CsvRow, MAX_CSV_ROWS, parse_csv_rows, sanitize_headers};
use super::open::ScratchDb;
use crate::HarnessError;
use crate::workspace::MAX_SCRATCH_IMPORT_BYTES;

/// Largest file the scratch importer accepts before parsing.
pub const MAX_IMPORT_FILE_BYTES: usize = MAX_SCRATCH_IMPORT_BYTES;
/// Wall-clock ceiling for parsing and insertion on the blocking import lane.
pub const IMPORT_TIMEOUT: Duration = Duration::from_secs(60);

/// An imported table's non-sensitive receipt.
#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub table: String,
    pub columns: Vec<String>,
    pub rows_imported: usize,
    pub bytes_read: usize,
}

/// Typed import refusals. None contain CSV field values.
#[derive(Debug, Error)]
pub enum ImportError {
    #[error("scratch import is unavailable without a bound workspace")]
    WorkspaceUnavailable,
    #[error("scratch import arguments are invalid")]
    InvalidArguments,
    #[error(
        "scratch import table must be a plain identifier of 1 to 63 ASCII letters, digits, or underscores"
    )]
    InvalidTable,
    #[error("scratch import delimiter must be one ASCII character")]
    InvalidDelimiter,
    #[error("scratch import if_exists must be \"fail\" or \"replace\"")]
    InvalidIfExists,
    #[error("scratch import has no header row")]
    MissingHeader,
    #[error("scratch import row {line} has {found} fields; expected {expected}")]
    RaggedRow {
        line: usize,
        found: usize,
        expected: usize,
    },
    #[error("scratch import source exceeds the {max}-byte limit")]
    FileTooLarge { max: usize },
    #[error("scratch import timed out")]
    TimedOut,
    #[error("scratch import failed")]
    Database,
    #[error("scratch import workspace file refused: {0}")]
    Workspace(#[from] HarnessError),
    #[error(transparent)]
    Csv(#[from] CsvError),
}

pub(crate) fn import_bytes(
    db: &ScratchDb,
    table: &str,
    bytes: &[u8],
    header: bool,
    delimiter: u8,
    replace: bool,
) -> Result<ImportResult, ImportError> {
    let started = Instant::now();
    if !plain_identifier(table) {
        return Err(ImportError::InvalidTable);
    }
    if !delimiter.is_ascii() {
        return Err(ImportError::InvalidDelimiter);
    }
    if bytes.len() > MAX_IMPORT_FILE_BYTES {
        return Err(ImportError::FileTooLarge {
            max: MAX_IMPORT_FILE_BYTES,
        });
    }
    let rows = parse_csv_rows(bytes, delimiter)?;
    let (columns, data) = if header {
        let (head, data) = rows.split_first().ok_or(ImportError::MissingHeader)?;
        (sanitize_headers(&head.fields), data)
    } else {
        let count = rows.first().map_or(0, |row| row.fields.len());
        (
            (1..=count).map(|i| format!("column_{i}")).collect(),
            rows.as_slice(),
        )
    };
    for row in data {
        if row.fields.len() != columns.len() {
            return Err(ImportError::RaggedRow {
                line: row.line,
                found: row.fields.len(),
                expected: columns.len(),
            });
        }
    }
    if data.len() > MAX_CSV_ROWS {
        return Err(CsvError::TooManyRows { max: MAX_CSV_ROWS }.into());
    }
    insert(db, table, &columns, data, bytes.len(), replace, started)
}

fn insert(
    db: &ScratchDb,
    table: &str,
    columns: &[String],
    rows: &[CsvRow],
    bytes_read: usize,
    replace: bool,
    started: Instant,
) -> Result<ImportResult, ImportError> {
    if started.elapsed() > IMPORT_TIMEOUT {
        return Err(ImportError::TimedOut);
    }
    let connection = db.connection.lock().map_err(|_| ImportError::Database)?;
    if started.elapsed() > IMPORT_TIMEOUT {
        return Err(ImportError::TimedOut);
    }
    let tx = duckdb::Transaction::new_unchecked(&connection).map_err(|_| ImportError::Database)?;
    let stage = staging_name();
    let columns_sql = columns
        .iter()
        .map(|column| format!("{} VARCHAR", quote_identifier(column)))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!("CREATE TABLE {stage} ({columns_sql})"))
        .map_err(|_| ImportError::Database)?;
    let placeholders = std::iter::repeat_n("?", columns.len())
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = tx
        .prepare(&format!("INSERT INTO {stage} VALUES ({placeholders})"))
        .map_err(|_| ImportError::Database)?;
    for row in rows {
        if started.elapsed() > IMPORT_TIMEOUT {
            return Err(ImportError::TimedOut);
        }
        statement
            .execute(params_from_iter(row.fields.iter()))
            .map_err(|_| ImportError::Database)?;
    }
    drop(statement);
    let target = quote_identifier(table);
    if replace {
        tx.execute_batch(&format!(
            "DROP TABLE IF EXISTS {target}; ALTER TABLE {stage} RENAME TO {target}"
        ))
        .map_err(|_| ImportError::Database)?;
    } else {
        tx.execute_batch(&format!("ALTER TABLE {stage} RENAME TO {target}"))
            .map_err(|_| ImportError::Database)?;
    }
    if started.elapsed() > IMPORT_TIMEOUT {
        return Err(ImportError::TimedOut);
    }
    tx.commit().map_err(|_| ImportError::Database)?;
    Ok(ImportResult {
        table: table.to_owned(),
        columns: columns.to_vec(),
        rows_imported: rows.len(),
        bytes_read,
    })
}

fn staging_name() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    format!("__saya_import_{nanos:x}")
}

fn plain_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    (1..=63).contains(&bytes.len())
        && (bytes[0].is_ascii_alphabetic() || bytes[0] == b'_')
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
