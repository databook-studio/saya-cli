//! The Parquet staging connection's fixed SQL: metadata checks before any
//! decode, the bounded `CREATE TABLE … AS SELECT * FROM read_parquet(?)`
//! into the attached destination, and the staged metadata table. Every
//! statement is fixed; the file path arrives as a bound parameter and the
//! only literal interpolations are saya-generated identifiers and caps.

use std::{path::Path, time::Instant};

use duckdb::{Connection, Transaction, params};

use super::{
    ParquetCaps, RESERVED_METADATA_TABLE, STAGED_DB_FILE, SourceFormat, StageError, StagedSource,
    parquet_budget,
    parquet_preview::{build_preview, describe_columns},
    parquet_stage::{Watchdog, check_deadline, deadline_or_database, quote_identifier, sql_string},
    read,
};

/// The fixed inputs of one staging run, threaded through the pipeline as a
/// unit: the locked staging connection, the destination, the private copy,
/// and the bounds.
pub(super) struct Staging<'a> {
    pub connection: &'a Connection,
    pub dest_dir: &'a Path,
    pub temp_db: &'a Path,
    pub copy: &'a Path,
    pub read: &'a read::SourceRead,
    pub table: &'a str,
    pub caps: ParquetCaps,
    pub deadline: Instant,
}

/// Runs the staging decode: metadata refusals first, then the transactional
/// write into a database attached at `temp_db`, then the preview reads. The
/// attached database is checkpointed and detached before returning, so the
/// rename that publishes `source.duckdb` never strands a `.wal` beside a
/// stale temp name.
pub(super) fn decode_and_write(stage: &Staging<'_>) -> Result<StagedSource, StageError> {
    // The watchdog interrupts a statement still running at the deadline —
    // the bounded decode's wall clock — and cancels itself on every exit.
    let _watchdog = Watchdog::arm(stage.connection.interrupt_handle(), stage.deadline);
    check_deadline(stage.deadline)?;
    let copy_path = stage.copy.to_string_lossy().into_owned();
    let declared_rows: u64 = stage
        .connection
        .query_row(
            "SELECT COALESCE(sum(num_rows), 0) FROM parquet_file_metadata(?)",
            params![copy_path],
            |row| row.get(0),
        )
        .map_err(|error| deadline_or_database(error, stage.deadline))?;
    if declared_rows > stage.caps.max_rows {
        return Err(StageError::ParquetTooManyRows {
            rows: declared_rows,
            max: stage.caps.max_rows,
        });
    }
    let columns = describe_columns(
        stage.connection,
        &copy_path,
        stage.caps.max_columns,
        stage.deadline,
    )?;
    stage
        .connection
        .execute_batch(&format!(
            "ATTACH {} AS out",
            sql_string(&stage.temp_db.to_string_lossy())
        ))
        .map_err(|error| deadline_or_database(error, stage.deadline))?;
    let tx = Transaction::new_unchecked(stage.connection).map_err(|_| StageError::Database)?;
    let staged_rows = write_table(&tx, stage, &copy_path, &columns);
    match staged_rows {
        Ok(staged_rows) => {
            check_deadline(stage.deadline)?;
            tx.commit().map_err(|_| StageError::Database)?;
            let preview = build_preview(stage.connection, stage.table, &columns, stage.deadline)?;
            checkpoint_and_detach(stage.connection, stage.deadline)?;
            Ok(StagedSource {
                db_path: stage.dest_dir.join(STAGED_DB_FILE),
                table: stage.table.to_owned(),
                columns: columns.iter().map(|(name, _)| name.clone()).collect(),
                rows: staged_rows as usize,
                sha256: stage.read.sha256.clone(),
                bytes: stage.read.size,
                preview,
                format: SourceFormat::Parquet,
            })
        }
        Err(error) => {
            let _ = tx.rollback();
            Err(error)
        }
    }
}

/// The bounded decode-and-write: `CREATE TABLE` with the cap-plus-one LIMIT,
/// a count check over what actually landed, and the metadata table — all in
/// the caller's transaction.
fn write_table(
    tx: &Transaction<'_>,
    stage: &Staging<'_>,
    copy_path: &str,
    columns: &[(String, String)],
) -> Result<u64, StageError> {
    tx.execute(
        &format!(
            "CREATE TABLE out.{} AS SELECT * FROM read_parquet(?) LIMIT {}",
            quote_identifier(stage.table),
            stage.caps.max_rows.saturating_add(1)
        ),
        params![copy_path],
    )
    .map_err(|error| deadline_or_database(error, stage.deadline))?;
    let staged_rows: u64 = tx
        .query_row(
            &format!("SELECT count(*) FROM out.{}", quote_identifier(stage.table)),
            [],
            |row| row.get(0),
        )
        .map_err(|error| deadline_or_database(error, stage.deadline))?;
    if staged_rows > stage.caps.max_rows {
        return Err(StageError::ParquetTooManyRows {
            rows: staged_rows,
            max: stage.caps.max_rows,
        });
    }
    let decoded_bytes = parquet_budget::decoded_cell_bytes(stage, tx, columns)?;
    if decoded_bytes > stage.caps.max_decoded_bytes {
        return Err(StageError::ParquetTooManyDecodedBytes {
            bytes: decoded_bytes,
            max: stage.caps.max_decoded_bytes,
        });
    }
    write_metadata(tx, stage.read, columns.len(), staged_rows)?;
    Ok(staged_rows)
}

fn write_metadata(
    tx: &Transaction<'_>,
    read: &read::SourceRead,
    column_count: usize,
    row_count: u64,
) -> Result<(), StageError> {
    tx.execute_batch(&format!(
        "CREATE TABLE out.{RESERVED_METADATA_TABLE} (key VARCHAR, value VARCHAR)"
    ))
    .map_err(|_| StageError::Database)?;
    let entries: [(&str, String); 7] = [
        ("file_name", read.file_name.clone()),
        ("sha256", read.sha256.clone()),
        ("bytes", read.size.to_string()),
        ("rows", row_count.to_string()),
        ("columns", column_count.to_string()),
        ("staged_unix_ms", staged_unix_ms().to_string()),
        ("format", "parquet".to_owned()),
    ];
    for (key, value) in entries.iter() {
        tx.execute(
            &format!("INSERT INTO out.{RESERVED_METADATA_TABLE} VALUES (?, ?)"),
            params![*key, value],
        )
        .map_err(|_| StageError::Database)?;
    }
    Ok(())
}

/// The staged-write wall-clock's timestamp key for the metadata table.
fn staged_unix_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

/// Forces the attached database's write-out and closes it cleanly, so the
/// rename that publishes `source.duckdb` never strands a `.wal` beside a
/// stale temp name.
fn checkpoint_and_detach(connection: &Connection, deadline: Instant) -> Result<(), StageError> {
    connection
        .execute_batch("CHECKPOINT out; DETACH out")
        .map_err(|error| deadline_or_database(error, deadline))
}
