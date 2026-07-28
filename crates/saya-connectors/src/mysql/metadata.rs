use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{MySqlConnector, errors};

const SCHEMA_SQL: &str = "SELECT table_name, column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema = ? ORDER BY table_name, ordinal_position";

pub(crate) async fn schema(connector: &MySqlConnector) -> Result<SchemaTree, ConnectionError> {
    let rows = timeout(
        connector.query_timeout,
        sqlx::query(SCHEMA_SQL)
            .bind(&connector.database)
            .fetch_all(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::SchemaFailed("MySQL schema discovery timed out".into()))?
    .map_err(errors::schema)?;
    let mut tables = BTreeMap::<String, Vec<Column>>::new();
    for row in rows {
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
