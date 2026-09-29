//! The staging writer: one private DuckDB database file built in a temp file
//! under the same pinned configuration as the scratch database (external
//! access off, extension autoload off, configuration locked), transactionally
//! committed, then renamed into place. Any failure removes the temp file and
//! its WAL — no partial `source.duckdb` is ever left behind.

use std::{
    fs,
    path::{Path, PathBuf},
    process,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt};

use duckdb::{AccessMode, Config, Connection, Transaction, params, params_from_iter};

use crate::workspace::contain::nanos;

use super::read::SourceRead;
use super::{RESERVED_METADATA_TABLE, StageError};

/// Rows between wall-clock deadline checks during insertion.
const DEADLINE_BATCH: usize = 256;

/// Guards the temp staging file: while armed, dropping removes the temp file
/// and its WAL, so every failure path — including panics — leaves nothing.
struct TempGuard {
    path: PathBuf,
    armed: bool,
}

impl TempGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for TempGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_file(wal_path(&self.path));
        }
    }
}

fn wal_path(path: &Path) -> PathBuf {
    let mut wal = path.as_os_str().to_os_string();
    wal.push(".wal");
    PathBuf::from(wal)
}

/// A unique temp name inside the destination directory, DuckDB-flavoured so
/// a `.wal` beside it is unambiguous.
fn temp_name() -> String {
    format!(".saya-filestage-{}-{}.duckdb", process::id(), nanos())
}

/// Writes the staged database into a temp file under `dest_dir` and renames
/// it to `source.duckdb` once committed. The deadline bounds the whole write.
pub(super) fn write_staged(
    dest_dir: &Path,
    table: &str,
    columns: &[String],
    rows: &[Vec<String>],
    source: &SourceRead,
    deadline: Instant,
) -> Result<(), StageError> {
    check_deadline(deadline)?;
    let temp = dest_dir.join(temp_name());
    let guard = TempGuard::new(temp.clone());
    let connection = open_connection(&temp)?;
    let written = write_all(&connection, table, columns, rows, source, deadline);
    drop(connection);
    written?;
    fs::rename(&temp, dest_dir.join(super::STAGED_DB_FILE))
        .map_err(|error| StageError::Destination(format!("commit staged file: {error}")))?;
    guard.disarm();
    Ok(())
}

fn check_deadline(deadline: Instant) -> Result<(), StageError> {
    if Instant::now() >= deadline {
        return Err(StageError::Timeout);
    }
    Ok(())
}

fn open_connection(path: &Path) -> Result<Connection, StageError> {
    let config = staged_config();
    let connection = Connection::open_with_flags(path, config).map_err(|_| StageError::Database)?;
    #[cfg(unix)]
    fs::set_permissions(path, Permissions::from_mode(0o600)).map_err(|error| {
        StageError::FileMode {
            path: path.to_path_buf(),
            source: error,
        }
    })?;
    Ok(connection)
}

/// The scratch configuration, pinned the same way `scratch/open.rs` pins the
/// run's scratch file (the module there is private, so it is restated here):
/// external access off, autoload and community extensions off, no persistent
/// secrets, configuration locked.
fn staged_config() -> Config {
    Config::default()
        .access_mode(AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("staging security configuration is valid")
}

fn write_all(
    connection: &Connection,
    table: &str,
    columns: &[String],
    rows: &[Vec<String>],
    source: &SourceRead,
    deadline: Instant,
) -> Result<(), StageError> {
    check_deadline(deadline)?;
    let tx = Transaction::new_unchecked(connection).map_err(|_| StageError::Database)?;
    let columns_sql = columns
        .iter()
        .map(|column| format!("{} VARCHAR", quote_identifier(column)))
        .collect::<Vec<_>>()
        .join(", ");
    tx.execute_batch(&format!(
        "CREATE TABLE {} ({columns_sql})",
        quote_identifier(table)
    ))
    .map_err(|_| StageError::Database)?;
    let placeholders = std::iter::repeat_n("?", columns.len())
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = tx
        .prepare(&format!(
            "INSERT INTO {} VALUES ({placeholders})",
            quote_identifier(table)
        ))
        .map_err(|_| StageError::Database)?;
    for (index, row) in rows.iter().enumerate() {
        if index % DEADLINE_BATCH == 0 {
            check_deadline(deadline)?;
        }
        // An empty CSV field is a missing value: it stages as NULL, matching
        // the preview's null counts.
        statement
            .execute(params_from_iter(row.iter().map(|field| {
                if field.is_empty() {
                    None::<&str>
                } else {
                    Some(field.as_str())
                }
            })))
            .map_err(|_| StageError::Database)?;
    }
    drop(statement);
    write_metadata(&tx, source, columns.len(), rows.len())?;
    check_deadline(deadline)?;
    tx.commit().map_err(|_| StageError::Database)?;
    Ok(())
}

fn write_metadata(
    tx: &Transaction<'_>,
    source: &SourceRead,
    column_count: usize,
    row_count: usize,
) -> Result<(), StageError> {
    tx.execute_batch(&format!(
        "CREATE TABLE {RESERVED_METADATA_TABLE} (key VARCHAR, value VARCHAR)"
    ))
    .map_err(|_| StageError::Database)?;
    let staged_unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let entries: [(&str, String); 7] = [
        ("file_name", source.file_name.clone()),
        ("sha256", source.sha256.clone()),
        ("bytes", source.size.to_string()),
        ("rows", row_count.to_string()),
        ("columns", column_count.to_string()),
        ("staged_unix_ms", staged_unix_ms.to_string()),
        ("format", "csv".to_owned()),
    ];
    for (key, value) in entries.iter() {
        tx.execute(
            &format!("INSERT INTO {RESERVED_METADATA_TABLE} VALUES (?, ?)"),
            params![*key, value],
        )
        .map_err(|_| StageError::Database)?;
    }
    Ok(())
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
