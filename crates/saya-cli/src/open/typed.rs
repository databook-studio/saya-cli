//! The `--typed` copy: a second table `<table>_typed` built by explicit
//! `TRY_CAST` per inferred non-text column, in a separate read-write
//! connection on the staged file (never the session's read-only profile),
//! transactionally, with the VARCHAR table untouched. Values that fail the
//! cast are counted and reported — never silently.

use std::path::Path;

use duckdb::{Connection, Transaction};
use saya_harness::file_source::{InferredType, PreviewColumn};

use super::snapshot::staged_config;

/// The typed-copy outcome: which columns were cast, which stayed text, and
/// per-column cast-failure counts (`failed of total` non-null values).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TypedReport {
    pub table: String,
    pub cast_columns: Vec<(String, &'static str)>,
    pub text_columns: Vec<String>,
    pub failures: Vec<(String, usize, usize)>,
}

fn cast_target(inferred: InferredType) -> Option<(&'static str, &'static str)> {
    match inferred {
        InferredType::Integer => Some(("BIGINT", "integer")),
        InferredType::Decimal => Some(("DOUBLE", "decimal")),
        InferredType::Boolean => Some(("BOOLEAN", "boolean")),
        InferredType::Date => Some(("DATE", "date")),
        InferredType::Timestamp => Some(("TIMESTAMP", "timestamp")),
        InferredType::Text => None,
    }
}

pub(super) fn build(
    db_path: &Path,
    table: &str,
    columns: &[PreviewColumn],
) -> Result<TypedReport, String> {
    if columns.is_empty() {
        return Err("the staged table has no columns".into());
    }
    let typed_table = format!("{table}_typed");
    let cast: Vec<(&str, (&'static str, &'static str))> = columns
        .iter()
        .filter_map(|column| {
            cast_target(column.inferred).map(|target| (column.name.as_str(), target))
        })
        .collect();
    let connection = Connection::open_with_flags(db_path, staged_config(false)?)
        .map_err(|_| format!("could not open {} for the typed copy", db_path.display()))?;
    let mut failures = Vec::new();
    for (name, (sql_type, _)) in &cast {
        failures.extend(count_failures(&connection, table, name, sql_type)?);
    }
    let projection = columns
        .iter()
        .map(|column| match cast_target(column.inferred) {
            Some((sql_type, _)) => format!(
                "TRY_CAST({} AS {sql_type}) AS {}",
                quote_identifier(&column.name),
                quote_identifier(&column.name)
            ),
            None => format!(
                "{} AS {}",
                quote_identifier(&column.name),
                quote_identifier(&column.name)
            ),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let tx = Transaction::new_unchecked(&connection)
        .map_err(|_| "could not start the typed-copy transaction".to_owned())?;
    let built = (|| -> Result<(), String> {
        tx.execute_batch(&format!(
            "DROP TABLE IF EXISTS {}",
            quote_identifier(&typed_table)
        ))
        .map_err(|_| format!("could not clear the previous typed copy {typed_table:?}"))?;
        tx.execute_batch(&format!(
            "CREATE TABLE {} AS SELECT {projection} FROM {}",
            quote_identifier(&typed_table),
            quote_identifier(table)
        ))
        .map_err(|_| format!("could not build the typed copy {typed_table:?}"))?;
        Ok(())
    })();
    match built {
        Ok(()) => tx
            .commit()
            .map_err(|_| "could not commit the typed copy".to_owned())?,
        Err(error) => {
            let _ = tx.rollback();
            return Err(error);
        }
    }
    Ok(TypedReport {
        table: typed_table,
        cast_columns: cast
            .iter()
            .map(|(name, (_, label))| ((*name).to_owned(), *label))
            .collect(),
        text_columns: columns
            .iter()
            .filter(|column| cast_target(column.inferred).is_none())
            .map(|column| column.name.clone())
            .collect(),
        failures,
    })
}

/// Counts, for one cast column, how many non-null values fail the cast —
/// measured against the VARCHAR table before the copy is built.
fn count_failures(
    connection: &Connection,
    table: &str,
    column: &str,
    sql_type: &str,
) -> Result<Option<(String, usize, usize)>, String> {
    let quoted = quote_identifier(column);
    let sql = format!(
        "SELECT count(*) FILTER (WHERE {quoted} IS NOT NULL \
         AND TRY_CAST({quoted} AS {sql_type}) IS NULL), \
         count(*) FILTER (WHERE {quoted} IS NOT NULL) FROM {}",
        quote_identifier(table)
    );
    let mut statement = connection
        .prepare(&sql)
        .map_err(|_| format!("could not count the cast results for {column:?}"))?;
    let (failed, total) = statement
        .query_row([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
        .map_err(|_| format!("could not count the cast results for {column:?}"))?;
    let failed = failed.max(0) as usize;
    if failed == 0 {
        return Ok(None);
    }
    Ok(Some((column.to_owned(), failed, total.max(0) as usize)))
}

fn quote_identifier(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
