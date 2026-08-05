use futures_util::TryStreamExt;
use saya_types::{ConnectionError, QueryRequest, QueryResult};
use serde_json::Value;
use sqlx::{Column as _, Row};
use tokio::time::timeout;

use super::{PostgresConnector, decode::json_value, errors};

pub(crate) async fn query(
    connector: &PostgresConnector,
    request: QueryRequest,
) -> Result<QueryResult, ConnectionError> {
    let sql = crate::prepare_postgres_sql(&request.sql, request.max_rows)?;
    let mut connection = timeout(connector.query_timeout, connector.pool.acquire())
        .await
        .map_err(|_| ConnectionError::ConnectionFailed("PostgreSQL connection timed out".into()))?
        .map_err(errors::connection)?;
    let pid = timeout(
        connector.query_timeout,
        sqlx::query_scalar("SELECT pg_backend_pid()").fetch_one(&mut *connection),
    )
    .await
    .map_err(|_| ConnectionError::QueryFailed("PostgreSQL query timed out".into()))?
    .map_err(errors::query)?;
    *connector.active_pid.lock().await = Some(pid);
    let result = collect(
        connector,
        &mut connection,
        &sql,
        request.max_rows,
        request.sql,
    )
    .await;
    *connector.active_pid.lock().await = None;
    result
}

async fn collect(
    connector: &PostgresConnector,
    connection: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    sql: &str,
    max_rows: usize,
    original_sql: String,
) -> Result<QueryResult, ConnectionError> {
    let work = async {
        let mut stream = sqlx::query(sql).fetch(&mut **connection);
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
            if rows.len() == max_rows {
                return Ok(QueryResult {
                    columns,
                    rows,
                    row_count: max_rows,
                    truncated: true,
                    executed_sql: original_sql,
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
            executed_sql: original_sql,
        })
    };
    timeout(connector.query_timeout, work)
        .await
        .map_err(|_| ConnectionError::QueryFailed("PostgreSQL query timed out".into()))?
        .map_err(errors::query)
}
