use futures_util::TryStreamExt;
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;
use sqlx::{Column as _, Row};
use tokio::time::timeout;

use super::{SqliteConnector, decode, errors};

pub(crate) async fn query(
    c: &SqliteConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_sqlite_sql(&request.sql, request.max_rows)?;
    let work = async {
        let mut stream = sqlx::query(&sql).fetch(&c.pool);
        let mut columns = Vec::new();
        let mut rows = Vec::new();
        while let Some(row) = stream.try_next().await? {
            if columns.is_empty() {
                columns = row
                    .columns()
                    .iter()
                    .map(|column| column.name().to_string())
                    .collect();
            }
            let row_values = (0..row.len())
                .map(|index| decode::json_value(&row, index))
                .collect::<Result<Vec<_>, _>>()?;
            rows.push(Value::Array(row_values));
        }
        Ok((columns, rows))
    };

    let (columns, rows) = timeout(c.query_timeout, work)
        .await
        .map_err(|_| ConnectionError::QueryFailed("SQLite query timed out".into()))?
        .map_err(errors::query)?;

    Ok(crate::common::bounded_result(
        columns,
        rows,
        request.max_rows,
        request.sql,
    ))
}
