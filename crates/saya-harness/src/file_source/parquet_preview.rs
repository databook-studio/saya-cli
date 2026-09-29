//! The Parquet staging reads that shape the preview and enforce the schema
//! bounds: column names and types from the file's footer (no row decode),
//! per-column null counts over the staged table, and sample rows rendered as
//! text. All statements are fixed, with saya-generated identifiers.

use std::time::Instant;

use duckdb::{Connection, params};

use super::{
    StageError,
    infer::InferredType,
    parquet_stage::{check_deadline, deadline_or_database, quote_identifier, truncate},
    preview::{MAX_SAMPLE_ROWS, Preview, PreviewColumn},
};

/// Reads column names and DuckDB types from the file's footer — no row
/// decode — refusing more than `max_columns` columns and any nested type.
pub(super) fn describe_columns(
    connection: &Connection,
    copy_path: &str,
    max_columns: usize,
    deadline: Instant,
) -> Result<Vec<(String, String)>, StageError> {
    let mut statement = connection
        .prepare("DESCRIBE SELECT * FROM read_parquet(?)")
        .map_err(|error| deadline_or_database(error, deadline))?;
    let mut rows = statement
        .query(params![copy_path])
        .map_err(|error| deadline_or_database(error, deadline))?;
    let mut columns = Vec::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| deadline_or_database(error, deadline))?
    {
        let name: String = row.get(0).map_err(|_| StageError::Database)?;
        let kind: String = row.get(1).map_err(|_| StageError::Database)?;
        if is_nested(&kind) {
            return Err(StageError::ParquetNestedColumn {
                column: name,
                kind: truncate(&kind, 120),
            });
        }
        columns.push((name, kind));
        if columns.len() > max_columns {
            return Err(StageError::ParquetTooManyColumns {
                columns: columns.len(),
                max: max_columns,
            });
        }
        check_deadline(deadline)?;
    }
    if columns.is_empty() {
        return Err(StageError::ParquetNoColumns);
    }
    Ok(columns)
}

fn is_nested(kind: &str) -> bool {
    kind.contains("STRUCT(")
        || kind.contains("MAP(")
        || kind.contains("UNION(")
        || kind.contains('[')
}

/// Maps a DuckDB column type to the preview's inferred-type label. Types
/// outside the label set (BLOB, TIME, UUID, INTERVAL, …) stay text.
fn inferred_type(kind: &str) -> InferredType {
    let upper = kind.to_ascii_uppercase();
    if upper.starts_with("DECIMAL")
        || upper.starts_with("FLOAT")
        || upper.starts_with("DOUBLE")
        || upper.starts_with("REAL")
    {
        InferredType::Decimal
    } else if upper.starts_with("BOOLEAN") {
        InferredType::Boolean
    } else if upper.starts_with("TIMESTAMP") {
        InferredType::Timestamp
    } else if upper.starts_with("DATE") {
        InferredType::Date
    } else if upper.contains("INT") {
        InferredType::Integer
    } else {
        InferredType::Text
    }
}

/// Assembles the staged Parquet's preview: one scan for per-column null
/// counts, up to five sample rows cast to text.
pub(super) fn build_preview(
    connection: &Connection,
    table: &str,
    columns: &[(String, String)],
    deadline: Instant,
) -> Result<Preview, StageError> {
    let nulls = null_counts(connection, table, columns, deadline)?;
    let samples = sample_rows(connection, table, columns, deadline)?;
    Ok(Preview {
        // Parquet columns are named and typed natively; the CSV-only preview
        // fields are inert for this format.
        delimiter: b'\0',
        header: true,
        columns: columns
            .iter()
            .zip(nulls)
            .map(|((name, kind), null_count)| PreviewColumn {
                name: name.clone(),
                null_count,
                inferred: inferred_type(kind),
            })
            .collect(),
        sample_rows: samples,
    })
}

fn null_counts(
    connection: &Connection,
    table: &str,
    columns: &[(String, String)],
    deadline: Instant,
) -> Result<Vec<usize>, StageError> {
    check_deadline(deadline)?;
    let projections: Vec<String> = columns
        .iter()
        .map(|(name, _)| format!("count(*) FILTER (WHERE {} IS NULL)", quote_identifier(name)))
        .collect();
    let sql = format!(
        "SELECT {} FROM out.{}",
        projections.join(", "),
        quote_identifier(table)
    );
    let mut statement = connection.prepare(&sql).map_err(|_| StageError::Database)?;
    statement
        .query_row([], |row| {
            Ok((0..columns.len())
                .map(|index| row.get::<_, i64>(index).unwrap_or(0).max(0) as usize)
                .collect())
        })
        .map_err(|_| StageError::Database)
}

fn sample_rows(
    connection: &Connection,
    table: &str,
    columns: &[(String, String)],
    deadline: Instant,
) -> Result<Vec<Vec<String>>, StageError> {
    check_deadline(deadline)?;
    let projections: Vec<String> = columns
        .iter()
        .map(|(name, _)| format!("CAST({} AS VARCHAR)", quote_identifier(name)))
        .collect();
    let sql = format!(
        "SELECT {} FROM out.{} LIMIT {MAX_SAMPLE_ROWS}",
        projections.join(", "),
        quote_identifier(table)
    );
    let mut statement = connection.prepare(&sql).map_err(|_| StageError::Database)?;
    let mut rows = statement.query([]).map_err(|_| StageError::Database)?;
    let mut samples = Vec::new();
    while let Some(row) = rows.next().map_err(|_| StageError::Database)? {
        samples.push(
            (0..columns.len())
                .map(|index| {
                    row.get::<_, Option<String>>(index)
                        .ok()
                        .flatten()
                        .unwrap_or_default()
                })
                .collect(),
        );
    }
    Ok(samples)
}
