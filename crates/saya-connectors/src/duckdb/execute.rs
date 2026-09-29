use duckdb::{params_from_iter, types::Value as DbValue};
use saya_types::{BoundParam, ConnectionError, QueryRequest, QueryResult, SqlDialect};
use serde_json::Value;
use tokio::time::timeout;

use super::{DuckDbConnector, decode::json_value, errors};

#[cfg(test)]
#[path = "execute_tests.rs"]
mod tests;

pub(crate) async fn ping(connector: &DuckDbConnector) -> Result<(), ConnectionError> {
    run(connector, Operation::Connection, |connection| {
        connection
            .execute_batch("SELECT 1")
            .map_err(errors::connection)
    })
    .await
}

pub(crate) async fn query(
    connector: &DuckDbConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let (sql, binds) = prepare(&request.sql, request.max_rows, &request.params)?;
    let original_sql = request.sql;
    let max_rows = request.max_rows;
    run(connector, Operation::Query, move |connection| {
        let mut statement = connection.prepare(&sql).map_err(errors::query)?;
        let mut rows = statement
            .query(params_from_iter(binds))
            .map_err(errors::query)?;
        let columns = rows
            .as_ref()
            .map(|statement| statement.column_names())
            .unwrap_or_default();
        let mut values = Vec::new();
        let mut result_bytes = 0;
        while let Some(row) = rows.next().map_err(errors::decode)? {
            if values.len() == max_rows {
                return Ok(QueryResult {
                    columns,
                    rows: values,
                    row_count: max_rows,
                    truncated: true,
                    executed_sql: original_sql,
                });
            }
            let cells: Vec<_> = (0..columns.len())
                .map(|index| row.get_ref(index).map(json_value))
                .collect::<Result<_, _>>()
                .map_err(errors::decode)?;
            let mut row_values = Vec::with_capacity(cells.len());
            for cell in cells {
                // Same per-cell and total-byte budgets as the other backends:
                // without them a single `repeat('A', 500000000)` cell
                // allocates hundreds of megabytes before the row check fires.
                let cell = crate::common::cap_cell(cell);
                result_bytes += crate::common::value_bytes(&cell);
                row_values.push(cell);
            }
            values.push(Value::Array(row_values));
            if result_bytes > crate::common::MAX_RESULT_BYTES {
                let count = values.len();
                return Ok(QueryResult {
                    columns,
                    rows: values,
                    row_count: count,
                    truncated: true,
                    executed_sql: original_sql,
                });
            }
        }
        Ok(QueryResult {
            row_count: values.len(),
            columns,
            rows: values,
            truncated: false,
            executed_sql: original_sql,
        })
    })
    .await
}

/// Prepares the statement for execution. Parameter-free SQL keeps today's
/// prepare path byte-for-byte; parameterized SQL is rewritten to `?` markers
/// with an ordered bind list — values never touch the text.
fn prepare(
    sql: &str,
    max_rows: usize,
    params: &[BoundParam],
) -> Result<(String, Vec<DbValue>), ConnectionError> {
    if params.is_empty() {
        return Ok((crate::prepare_duckdb_sql(sql, max_rows)?, Vec::new()));
    }
    let prepared = crate::prepare_with_params(sql, max_rows, SqlDialect::DuckDb, params)?;
    let parsed = crate::binds::parse_bind_values(&prepared.values)?;
    let binds = super::bind::native_values(&parsed)?;
    Ok((prepared.sql, binds))
}

pub(crate) async fn run<T: Send + 'static>(
    connector: &DuckDbConnector,
    operation: Operation,
    work: impl FnOnce(&duckdb::Connection) -> Result<T, ConnectionError> + Send + 'static,
) -> Result<T, ConnectionError> {
    let connection = connector.connection.clone();
    let mut task = tokio::task::spawn_blocking(move || {
        let connection = connection
            .lock()
            .map_err(|_| operation.failed("connection lock failed"))?;
        work(&connection)
    });
    match timeout(connector.query_timeout, &mut task).await {
        Ok(result) => result.map_err(|_| operation.failed("task failed"))?,
        Err(_) => {
            connector.interrupt.interrupt();
            // `JoinHandle::abort` cannot stop blocking native work. DuckDB's interrupt handle
            // is the cancellation primitive; awaiting it proves the query released the mutex.
            let _ = task.await;
            Err(operation.failed("timed out"))
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum Operation {
    Connection,
    Schema,
    Query,
}

impl Operation {
    fn failed(self, detail: &str) -> ConnectionError {
        let message = format!("DuckDB {detail}");
        match self {
            Self::Connection => ConnectionError::connection_failed(message),
            Self::Schema => ConnectionError::schema_failed(message),
            Self::Query => ConnectionError::query_failed(message),
        }
    }
}
