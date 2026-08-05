use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;
use tokio::time::timeout;

use super::{DuckDbConnector, decode::json_value};

pub(crate) async fn ping(connector: &DuckDbConnector) -> Result<(), ConnectionError> {
    run(connector, Operation::Connection, |connection| {
        connection
            .execute_batch("SELECT 1")
            .map_err(connection_error)
    })
    .await
}

pub(crate) async fn query(
    connector: &DuckDbConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_duckdb_sql(&request.sql, request.max_rows)?;
    let original_sql = request.sql;
    let max_rows = request.max_rows;
    run(connector, Operation::Query, move |connection| {
        let mut statement = connection.prepare(&sql).map_err(error)?;
        let mut rows = statement.query([]).map_err(error)?;
        let columns = rows
            .as_ref()
            .map(|statement| statement.column_names())
            .unwrap_or_default();
        let mut values = Vec::new();
        while let Some(row) = rows.next().map_err(error)? {
            if values.len() == max_rows {
                return Ok(QueryResult {
                    columns,
                    rows: values,
                    row_count: max_rows,
                    truncated: true,
                    executed_sql: original_sql,
                });
            }
            values.push(Value::Array(
                (0..columns.len())
                    .map(|index| row.get_ref(index).map(json_value))
                    .collect::<Result<_, _>>()
                    .map_err(error)?,
            ));
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

fn error(_: duckdb::Error) -> ConnectionError {
    ConnectionError::QueryFailed("DuckDB query failed".into())
}

fn connection_error(_: duckdb::Error) -> ConnectionError {
    ConnectionError::ConnectionFailed("DuckDB connection failed".into())
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
            Self::Connection => ConnectionError::ConnectionFailed(message),
            Self::Schema => ConnectionError::SchemaFailed(message),
            Self::Query => ConnectionError::QueryFailed(message),
        }
    }
}
