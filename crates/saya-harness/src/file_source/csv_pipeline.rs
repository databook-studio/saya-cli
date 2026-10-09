//! Parsing, validation, the transactional write, and the staged receipt for
//! one already-contained CSV read.

use std::path::Path;
use std::time::Instant;

use crate::scratch::{CsvError, MAX_CSV_ROWS, parse_csv, sanitize_headers};

use super::read;
use super::{
    CsvStageOptions, RESERVED_METADATA_TABLE, STAGE_TIMEOUT, STAGED_DB_FILE, SourceFormat,
    StageError, StagedSource, csv_stage, prepare_dest_dir, preview, table_name,
};

/// The CSV pipeline from an already-contained read, shared by `stage_csv`
/// and the CSV arm of `stage_source`.
pub(super) fn stage_csv_read(
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
