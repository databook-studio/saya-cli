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
    let work = async {
        let mut stream = sqlx::query(&sql).fetch(&connector.pool);
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        while let Some(row) = stream.try_next().await? {
            if columns.is_empty() {
                columns = row
                    .columns()
                    .iter()
                    .map(|column| column.name().into())
                    .collect();
            }
            if rows.len() == request.max_rows {
                return Ok(QueryResult {
                    columns,
                    rows,
                    row_count: request.max_rows,
                    truncated: true,
                    executed_sql: request.sql,
                });
            }
            rows.push(Value::Array(
                (0..row.len())
                    .map(|index| json_value(&row, index))
                    .collect::<Result<_, _>>()?,
            ));
        }
        Ok(QueryResult {
            row_count: rows.len(),
            columns,
            rows,
            truncated: false,
            executed_sql: request.sql,
        })
    };
    timeout(connector.query_timeout, work)
        .await
        .map_err(|_| ConnectionError::QueryFailed("MySQL query timed out".into()))?
        .map_err(errors::query)
}
