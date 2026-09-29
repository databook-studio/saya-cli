//! The Parquet decoded-byte budget: what one staging may decode is bounded
//! not just by rows and columns but by accounted decoded bytes — the sum
//! over every source cell of its decoded size, text columns by bit length,
//! blob columns by octet length, every other flat type at its fixed
//! physical width. The accounting is a streaming aggregate straight over
//! the source copy — it materializes nothing — and runs BEFORE the decode's
//! CREATE TABLE: exceeding the budget refuses the whole staging with the
//! limit named, and no partial snapshot is ever visible.

use duckdb::params;

use super::{
    StageError,
    parquet_decode::Staging,
    parquet_stage::{check_deadline, deadline_or_database, quote_identifier},
};

/// The enforced decoded-byte budget for one Parquet staging.
pub(super) const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

/// The source's accounted decoded size in bytes, measured by a streaming
/// aggregate over `read_parquet(?)` — before any table is created.
pub(super) fn source_decoded_bytes(
    stage: &Staging<'_>,
    columns: &[(String, String)],
) -> Result<u64, StageError> {
    check_deadline(stage.deadline)?;
    let projections: Vec<String> = columns
        .iter()
        .map(|(name, kind)| cell_bytes_projection(name, kind))
        .collect();
    let sql = format!("SELECT {} FROM read_parquet(?)", projections.join(", "));
    let copy_path = stage.copy.to_string_lossy().into_owned();
    let total: u128 = stage
        .connection
        .query_row(&sql, params![copy_path], |row| {
            let mut total: u128 = 0;
            for index in 0..columns.len() {
                let cell: i64 = row.get(index).unwrap_or(0);
                total += cell.max(0) as u128;
            }
            Ok(total)
        })
        .map_err(|error| deadline_or_database(error, stage.deadline))?;
    Ok(u64::try_from(total).unwrap_or(u64::MAX))
}

/// One column's accounted-bytes projection: blob by octet length; text by
/// bit length (8 × the UTF-8 byte count — each cell's bit length is a
/// multiple of 8, so the integer division is exact); every other type at
/// its fixed physical width. All forms are streaming aggregates over the
/// scan — no cell payload is copied or materialized.
fn cell_bytes_projection(name: &str, kind: &str) -> String {
    let identifier = quote_identifier(name);
    let upper = kind.to_ascii_uppercase();
    if upper.starts_with("BLOB") {
        format!("CAST(COALESCE(sum(octet_length({identifier})), 0) AS BIGINT)")
    } else if upper.starts_with("VARCHAR") {
        format!("CAST(COALESCE(sum(bit_length({identifier})), 0) AS BIGINT) // 8")
    } else {
        format!(
            "CAST(count({identifier}) AS BIGINT) * {}",
            fixed_width(kind)
        )
    }
}

/// A flat column type's physical width in bytes, matching DuckDB's storage;
/// an unrecognised type is accounted at DuckDB's widest physical cell
/// (16 bytes) — under-counting is the one wrong direction for a budget.
fn fixed_width(kind: &str) -> u64 {
    let upper = kind.to_ascii_uppercase();
    if let Some(rest) = upper.strip_prefix("DECIMAL(") {
        let precision = rest
            .split(',')
            .next()
            .and_then(|item| item.trim().parse::<u64>().ok())
            .unwrap_or(38);
        return if precision <= 4 {
            2
        } else if precision <= 9 {
            4
        } else if precision <= 18 {
            8
        } else {
            16
        };
    }
    match upper.as_str() {
        "BOOLEAN" => 1,
        "TINYINT" | "UTINYINT" => 1,
        "SMALLINT" | "USMALLINT" => 2,
        "INTEGER" | "UINTEGER" => 4,
        "BIGINT" | "UBIGINT" => 8,
        "FLOAT" | "REAL" => 4,
        "DOUBLE" => 8,
        "DATE" => 4,
        "TIME" | "TIMESTAMP" | "TIMESTAMP WITH TIME ZONE" | "TIMESTAMPTZ" => 8,
        "HUGEINT" | "UHUGEINT" | "UUID" | "INTERVAL" => 16,
        _ => 16,
    }
}
