use futures_util::TryStreamExt;
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;
use sqlx::{Column as _, Row};
use tokio::time::timeout;

use super::{MySqlConnector, decode::json_value, errors};

pub(crate) async fn query(
    connector: &MySqlConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_mysql_sql(&request.sql, request.max_rows)?;
    // Serialize executes so `active_id` is unambiguous for cancellation.
    let _in_flight = connector.in_flight.lock().await;
    let id: u64 = timeout(
        connector.query_timeout,
        sqlx::query_scalar("SELECT CONNECTION_ID()").fetch_one(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::connection_failed("MySQL connection timed out"))?
    .map_err(errors::connection)?;
    *connector.active_id.lock().await = Some(id);
    let work = async {
        let mut stream = sqlx::query(&sql).fetch(&connector.pool);
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        let mut result_bytes = 0;
        let mut truncated = false;
        while let Some(row) = stream.try_next().await? {
            if columns.is_empty() {
                columns = row
                    .columns()
                    .iter()
                    .map(|column| column.name().into())
                    .collect();
            }
            if rows.len() == request.max_rows {
                truncated = true;
                break;
            }
            let mut row_values = Vec::with_capacity(row.len());
            for index in 0..row.len() {
                let cell = json_value(&row, index)?;
                let cell = crate::common::cap_cell(cell);
                result_bytes += crate::common::value_bytes(&cell);
                row_values.push(cell);
            }
            rows.push(Value::Array(row_values));

            if result_bytes > crate::common::MAX_RESULT_BYTES {
                truncated = true;
                break;
            }
        }
        Ok(QueryResult {
            row_count: rows.len(),
            columns,
            rows,
            truncated,
            executed_sql: request.sql,
        })
    };
    let result = timeout(connector.query_timeout, work).await;
    *connector.active_id.lock().await = None;
    // A timed-out MySQL query keeps running server-side unless killed; the
    // client-side wait alone would strand it on the pool connection.
    if result.is_err() {
        super::cancellation::cancel(connector).await.ok();
        return Err(ConnectionError::query_failed("MySQL query timed out"));
    }
    result.expect("checked above").map_err(errors::query)
}
