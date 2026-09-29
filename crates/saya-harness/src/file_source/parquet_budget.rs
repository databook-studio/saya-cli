//! The Parquet decoded-byte budget: what one staging may decode is bounded
//! not just by rows and columns but by accounted decoded bytes — the sum
//! over every staged cell of its decoded size, text and blob columns by
//! octet length, every other flat type at its fixed physical width. The
//! accounting runs inside the staging transaction over the staged table,
//! before any commit; exceeding the budget refuses the whole staging, and
//! the rollback leaves no visible snapshot.

use duckdb::Transaction;

use super::{
    StageError,
    parquet_decode::Staging,
    parquet_stage::{check_deadline, deadline_or_database, quote_identifier},
};

/// The enforced decoded-byte budget for one Parquet staging.
pub(super) const MAX_DECODED_BYTES: u64 = 64 * 1024 * 1024;

/// The staged table's accounted decoded size in bytes.
pub(super) fn decoded_cell_bytes(
    stage: &Staging<'_>,
    tx: &Transaction<'_>,
    columns: &[(String, String)],
) -> Result<u64, StageError> {
    check_deadline(stage.deadline)?;
    let projections: Vec<String> = columns
        .iter()
        .map(|(name, kind)| {
            let identifier = quote_identifier(name);
            if has_decoded_length(kind) {
                format!(
                    "CAST(COALESCE(sum(octet_length(CAST({identifier} AS BLOB))), 0) AS BIGINT)"
                )
            } else {
                format!(
                    "CAST(count({identifier}) AS BIGINT) * {}",
                    fixed_width(kind)
                )
            }
        })
        .collect();
    let sql = format!(
        "SELECT {} FROM out.{}",
        projections.join(", "),
        quote_identifier(stage.table)
    );
    let total: u128 = tx
        .query_row(&sql, [], |row| {
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

/// Whether a column type's cells carry their own decoded length; every
/// other type occupies a fixed width per non-NULL cell.
fn has_decoded_length(kind: &str) -> bool {
    let upper = kind.to_ascii_uppercase();
    upper.starts_with("VARCHAR") || upper.starts_with("BLOB")
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
