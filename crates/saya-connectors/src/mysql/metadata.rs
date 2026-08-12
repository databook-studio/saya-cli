use std::collections::BTreeMap;

use futures_util::TryStreamExt;
use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{MySqlConnector, errors};

const SCHEMA_SQL: &str = "SELECT CAST(TABLE_NAME AS CHAR) AS table_name, CAST(COLUMN_NAME AS CHAR) AS column_name, CAST(DATA_TYPE AS CHAR) AS data_type, CAST(IS_NULLABLE AS CHAR) AS is_nullable FROM information_schema.columns WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION";

pub(crate) async fn schema(connector: &MySqlConnector) -> Result<SchemaTree, ConnectionError> {
    let work = async {
        let mut stream = sqlx::query(SCHEMA_SQL)
            .bind(&connector.database)
            .fetch(&connector.pool);
        let mut tables = BTreeMap::<String, Vec<Column>>::new();
        while let Some(row) = stream.try_next().await.map_err(errors::schema)? {
            let table: String = row.try_get("table_name").map_err(errors::schema)?;
            let column = Column {
                name: row.try_get("column_name").map_err(errors::schema)?,
                data_type: row.try_get("data_type").map_err(errors::schema)?,
                nullable: row
                    .try_get::<String, _>("is_nullable")
                    .map_err(errors::schema)?
                    == "YES",
            };
            tables.entry(table).or_default().push(column);
        }
        Ok(tables)
    };
    let tables = timeout(connector.query_timeout, work)
        .await
        .map_err(|_| ConnectionError::schema_failed("MySQL schema discovery timed out"))??;
    let tables = tables
        .into_iter()
        .map(|(name, columns)| Table { name, columns })
        .collect();
    Ok(SchemaTree {
        databases: vec![Database {
            name: "MySQL".into(),
            schemas: vec![Schema {
                name: connector.database.clone(),
                tables,
            }],
        }],
    })
}
