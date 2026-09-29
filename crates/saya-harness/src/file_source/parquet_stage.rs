//! C2a Parquet staging: one local Parquet file decoded into the same private
//! DuckDB source layout as CSV, through a dedicated throwaway staging
//! connection — never through a `DatabaseConnector`, never scratch, never the
//! session.
//!
//! The contained single-file read (≤32 MiB, sha256, no-follow) hands over the
//! bytes; they are written to a saya-owned copy INSIDE the destination
//! directory (0600) and DuckDB is pointed at that copy only. The fixed SQL
//! and the preview reads live in [`super::parquet_decode`] and
//! [`super::parquet_preview`]. The staging connection is in-memory, pinned
//! and locked at open (external access must be set there — DuckDB refuses to
//! change it on a running database); a watchdog interrupts a statement still
//! running at the deadline; every cancel or failure deletes every temp file.

use std::{
    fs::{self, OpenOptions},
    io,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use duckdb::{AccessMode, Config, Connection, InterruptHandle};

use crate::workspace::contain::nanos;

use super::{
    RESERVED_METADATA_TABLE, STAGE_TIMEOUT, STAGED_DB_FILE, StageError, StagedSource,
    parquet_decode, read, table_name,
};

/// Decode and wall-clock caps for one Parquet staging: the published bounds
/// by default, shrinkable through these seams in tests.
#[derive(Debug, Clone, Copy)]
pub struct ParquetCaps {
    pub max_rows: u64,
    pub max_columns: usize,
    pub timeout: Duration,
}

impl Default for ParquetCaps {
    fn default() -> Self {
        Self {
            max_rows: 500_000,
            max_columns: 512,
            timeout: STAGE_TIMEOUT,
        }
    }
}

/// Stages one local Parquet file into a private DuckDB database file at
/// `<dest_dir>/source.duckdb`, bounded by the default caps.
pub fn stage_parquet(source: &Path, dest_dir: &Path) -> Result<StagedSource, StageError> {
    let read = read::read_source(source)?;
    stage_read(read, dest_dir, ParquetCaps::default())
}

/// Whether the already-contained read looks like Parquet: the `.parquet`
/// extension (case-insensitive) or the PAR1 magic at either end of the file.
pub(super) fn looks_like_parquet(read: &read::SourceRead) -> bool {
    read.file_name.to_ascii_lowercase().ends_with(".parquet")
        || read.bytes.starts_with(b"PAR1")
        || read.bytes.ends_with(b"PAR1")
}

/// The staging pipeline from an already-contained read, with the caps seam
/// tests use to shrink the row/column caps and the wall clock.
pub(super) fn stage_read(
    read: read::SourceRead,
    dest_dir: &Path,
    caps: ParquetCaps,
) -> Result<StagedSource, StageError> {
    let deadline = Instant::now() + caps.timeout;
    let table = table_name(&read.stem);
    if table == RESERVED_METADATA_TABLE {
        return Err(StageError::ReservedTableName { name: table });
    }
    super::prepare_dest_dir(dest_dir)?;
    let copy = dest_dir.join(copy_name());
    write_copy(&copy, &read.bytes)?;
    let temp_db = dest_dir.join(db_temp_name());
    let temps = TempFiles::new([copy.clone(), temp_db.clone(), wal_path(&temp_db)]);
    let connection = Connection::open_in_memory_with_flags(staging_config()?)
        .map_err(|_| StageError::Database)?;
    let staging = parquet_decode::Staging {
        connection: &connection,
        dest_dir,
        temp_db: &temp_db,
        copy: &copy,
        read: &read,
        table: &table,
        caps,
        deadline,
    };
    let staged = parquet_decode::decode_and_write(&staging)?;
    drop(connection);
    #[cfg(unix)]
    fs::set_permissions(&temp_db, fs::Permissions::from_mode(0o600)).map_err(|error| {
        StageError::FileMode {
            path: temp_db.clone(),
            source: error,
        }
    })?;
    fs::rename(&temp_db, dest_dir.join(STAGED_DB_FILE))
        .map_err(|error| StageError::Destination(format!("commit staged file: {error}")))?;
    drop(temps);
    Ok(staged)
}

fn write_copy(path: &Path, bytes: &[u8]) -> Result<(), StageError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options
        .open(path)
        .map_err(|error| StageError::Destination(format!("stage private copy: {error}")))?;
    io::Write::write_all(&mut file, bytes)
        .map_err(|error| StageError::Destination(format!("write private copy: {error}")))?;
    Ok(())
}

/// The staging connection's pinned configuration. External access is enabled
/// because reading the private copy and attaching the destination are file
/// operations; DuckDB refuses to change the setting on a running database, so
/// it is pinned at open together with the rest and locked immediately.
fn staging_config() -> Result<Config, StageError> {
    Config::default()
        .access_mode(AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(true))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("memory_limit", "256MB"))
        .and_then(|item| item.with("threads", "2"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .map_err(|_| StageError::Database)
}

/// Interrupts a statement still running at the deadline, from another thread.
/// Dropping cancels and joins the thread, so the interrupt handle never
/// outlives the connection — on every path, including panics.
pub(super) struct Watchdog {
    cancel: mpsc::Sender<()>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Watchdog {
    pub(super) fn arm(interrupt: Arc<InterruptHandle>, deadline: Instant) -> Self {
        let (cancel, gate) = mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if gate.recv_timeout(remaining).is_err() {
                interrupt.interrupt();
            }
        });
        Self {
            cancel,
            handle: Some(handle),
        }
    }
}

impl Drop for Watchdog {
    fn drop(&mut self) {
        let _ = self.cancel.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Removes every temp artifact (the private Parquet copy, the destination
/// temp database, and its WAL) on all paths — success included, because the
/// copy's job ends with the decode and the temp database has been renamed.
struct TempFiles {
    paths: Vec<PathBuf>,
}

impl TempFiles {
    fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
        }
    }
}

impl Drop for TempFiles {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// A database error past the deadline is the wall-clock refusal; any other
/// database failure carries the engine's first error line — the decode only
/// ever sees saya-owned paths, never the user's original file.
pub(super) fn deadline_or_database(error: duckdb::Error, deadline: Instant) -> StageError {
    if Instant::now() >= deadline {
        StageError::Timeout
    } else {
        StageError::ParquetFailed(truncate(&error.to_string(), 300))
    }
}

pub(super) fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        value.to_owned()
    } else {
        value.chars().take(limit).collect()
    }
}

pub(super) fn check_deadline(deadline: Instant) -> Result<(), StageError> {
    if Instant::now() >= deadline {
        return Err(StageError::Timeout);
    }
    Ok(())
}

pub(super) fn sql_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(super) fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn copy_name() -> String {
    format!(".saya-parquet-{}-{}.parquet", process::id(), nanos())
}

fn db_temp_name() -> String {
    format!(".saya-filestage-{}-{}.duckdb", process::id(), nanos())
}

fn wal_path(path: &Path) -> PathBuf {
    let mut wal = path.as_os_str().to_os_string();
    wal.push(".wal");
    PathBuf::from(wal)
}
