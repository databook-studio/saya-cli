//! Scratch's execution engine: the same timeout, interrupt, and
//! await-on-cancel shape as the connector
//! (`saya-connectors/src/duckdb/execute.rs:90-99`) and the same row and byte
//! budgets on what reaches the model — re-implemented here, deliberately, on
//! its own types: scratch never shares connector plumbing (ADR 0003).

use duckdb::Connection;
use saya_types::QueryResult;
use serde_json::Value;
use tokio::time::timeout;

use super::ScratchError;
use super::decode::json_value;
use super::open::ScratchDb;
use super::validate::{SCRATCH_ROW_CAP, Validated};

/// Same per-cell and total-byte budgets as the other backends: without them a
/// single `repeat('A', 500000000)` cell allocates hundreds of megabytes
/// before the row check fires.
const MAX_CELL_BYTES: usize = 1 << 20;
const MAX_RESULT_BYTES: usize = 16 << 20;

/// The longest detail carried back. DuckDB echoes the offending statement
/// fragment in some messages, so the text is bounded like any other untrusted
/// input rather than trusted to stay short.
const MAX_DETAIL: usize = 300;

/// The DuckDB message classes that describe a fault in the *submitted SQL* —
/// a name that does not resolve, a binder mismatch, a parse error — rather
/// than a fault in stored data. Their text names identifiers the caller
/// itself supplied, so forwarding it discloses nothing the caller did not
/// already hold. Every other class is redacted: its text may echo a row value.
const EXPLAINABLE: [&str; 3] = ["Catalog Error:", "Binder Error:", "Parser Error:"];

impl ScratchDb {
    /// Runs one validated statement. Timed out, the DuckDB interrupt handle is
    /// the cancellation primitive — `JoinHandle::abort` cannot stop blocking
    /// native work — and awaiting the task proves the query released the
    /// connection mutex before the timeout error is surfaced.
    pub(super) async fn execute(
        &self,
        validated: Validated,
        original_sql: &str,
    ) -> Result<QueryResult, ScratchError> {
        let connection = self.connection.clone();
        let sql = validated.sql;
        let original = original_sql.to_string();
        let returns_rows = validated.returns_rows;
        let mut task = tokio::task::spawn_blocking(move || {
            let connection = connection.lock().map_err(|_| ScratchError::Execution {
                message: "connection lock failed".into(),
            })?;
            if returns_rows {
                collect_rows(&connection, &sql, &original)
            } else {
                run_effect(&connection, &sql, &original)
            }
        });
        match timeout(self.query_timeout, &mut task).await {
            Ok(result) => result.map_err(|_| ScratchError::Execution {
                message: "statement task failed".into(),
            })?,
            Err(_) => {
                self.interrupt.interrupt();
                // Awaiting proves the interrupted query released the mutex
                // before the timeout error is surfaced.
                let _ = task.await;
                Err(ScratchError::TimedOut)
            }
        }
    }
}

/// The no-rows path: the statement mutates the scratch file and reports how
/// many rows it changed.
fn run_effect(
    connection: &Connection,
    sql: &str,
    original: &str,
) -> Result<QueryResult, ScratchError> {
    let affected = connection
        .execute(sql, duckdb::params![])
        .map_err(scratch_query)?;
    Ok(QueryResult {
        columns: Vec::new(),
        rows: Vec::new(),
        row_count: affected,
        truncated: false,
        executed_sql: original.to_string(),
    })
}

/// The rows path: bounded collection at the row cap and byte budgets, with
/// `truncated` reporting what was cut rather than silently dropping it.
fn collect_rows(
    connection: &Connection,
    sql: &str,
    original: &str,
) -> Result<QueryResult, ScratchError> {
    let mut statement = connection.prepare(sql).map_err(scratch_query)?;
    let mut rows = statement.query([]).map_err(scratch_query)?;
    let columns = rows
        .as_ref()
        .map(|statement| statement.column_names())
        .unwrap_or_default();
    let mut values = Vec::new();
    let mut result_bytes = 0;
    while let Some(row) = rows.next().map_err(scratch_decode)? {
        if values.len() == SCRATCH_ROW_CAP {
            return Ok(result(columns, values, true, original));
        }
        let cells: Vec<_> = (0..columns.len())
            .map(|index| row.get_ref(index).map(json_value))
            .collect::<Result<_, _>>()
            .map_err(scratch_decode)?;
        let mut row_values = Vec::with_capacity(cells.len());
        for cell in cells {
            let cell = cap_cell(cell);
            result_bytes += value_bytes(&cell);
            row_values.push(cell);
        }
        values.push(Value::Array(row_values));
        if result_bytes > MAX_RESULT_BYTES {
            return Ok(result(columns, values, true, original));
        }
    }
    Ok(result(columns, values, false, original))
}

fn result(columns: Vec<String>, rows: Vec<Value>, truncated: bool, original: &str) -> QueryResult {
    QueryResult {
        row_count: rows.len(),
        columns,
        rows,
        truncated,
        executed_sql: original.to_string(),
    }
}

/// Maps a query-time DuckDB error: a message whose class is allow-listed is
/// forwarded, bounded; everything else is replaced so an unanticipated class
/// cannot carry a staged value back to the caller. A decode failure can carry
/// a staged value, so it is never forwarded at all.
fn scratch_query(error: duckdb::Error) -> ScratchError {
    let detail = match &error {
        duckdb::Error::DuckDBFailure(_, Some(message))
            if EXPLAINABLE.iter().any(|class| message.starts_with(class)) =>
        {
            bounded(message)
        }
        _ => "statement failed".to_string(),
    };
    ScratchError::Execution { message: detail }
}

/// Truncates on a character boundary so a multi-byte message cannot panic.
fn bounded(message: &str) -> String {
    if message.chars().count() <= MAX_DETAIL {
        return message.to_owned();
    }
    let kept: String = message.chars().take(MAX_DETAIL).collect();
    format!("{kept}…")
}

/// A decode failure can carry a staged value, so it is never forwarded.
fn scratch_decode(_: duckdb::Error) -> ScratchError {
    ScratchError::Execution {
        message: "statement failed".to_string(),
    }
}

/// Same per-cell cap as the other backends: a string cell over the budget is
/// truncated on a character boundary with a visible marker.
fn cap_cell(v: Value) -> Value {
    match v {
        Value::String(s) => {
            if s.len() > MAX_CELL_BYTES {
                let mut cut = MAX_CELL_BYTES;
                while !s.is_char_boundary(cut) {
                    cut -= 1;
                }
                let truncated_bytes = s.len() - cut;
                let slice = &s[..cut];
                Value::String(format!("{slice}…[truncated {truncated_bytes} bytes]"))
            } else {
                Value::String(s)
            }
        }
        Value::Array(arr) => Value::Array(arr.into_iter().map(cap_cell).collect()),
        Value::Object(map) => {
            Value::Object(map.into_iter().map(|(k, v)| (k, cap_cell(v))).collect())
        }
        other => other,
    }
}

fn value_bytes(v: &Value) -> usize {
    match v {
        Value::Null | Value::Bool(_) | Value::Number(_) => 8,
        Value::String(s) => s.len(),
        Value::Array(arr) => arr.iter().map(value_bytes).sum(),
        Value::Object(map) => map.iter().map(|(k, v)| k.len() + value_bytes(v)).sum(),
    }
}
