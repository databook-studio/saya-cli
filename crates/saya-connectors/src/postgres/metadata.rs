use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{PostgresConnector, errors};

pub(crate) async fn schema(connector: &PostgresConnector) -> Result<SchemaTree, ConnectionError> {
    let database = timeout(
        connector.query_timeout,
        sqlx::query_scalar::<_, String>("SELECT current_database()").fetch_one(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::SchemaFailed("PostgreSQL schema discovery timed out".into()))?
    .map_err(errors::schema)?;
    let rows = timeout(
        connector.query_timeout,
        sqlx::query(SCHEMA_SQL).fetch_all(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::SchemaFailed("PostgreSQL schema discovery timed out".into()))?
    .map_err(errors::schema)?;
    let mut schemas = BTreeMap::<String, BTreeMap<String, Vec<Column>>>::new();
    for row in rows {
        let schema = row.try_get("table_schema").map_err(errors::row)?;
        let table = row.try_get("table_name").map_err(errors::row)?;
        let column = Column {
            name: row.try_get("column_name").map_err(errors::row)?,
            data_type: row.try_get("data_type").map_err(errors::row)?,
            nullable: row
                .try_get::<String, _>("is_nullable")
                .map_err(errors::row)?
                == "YES",
        };
        schemas
            .entry(schema)
            .or_default()
            .entry(table)
            .or_default()
            .push(column);
    }
    let schemas = schemas
        .into_iter()
        .map(|(name, tables)| Schema {
            name,
            tables: tables
                .into_iter()
                .map(|(name, columns)| Table { name, columns })
                .collect(),
        })
        .collect();
    Ok(SchemaTree {
        databases: vec![Database {
            name: database,
            schemas,
        }],
    })
}

const SCHEMA_SQL: &str = "SELECT c.table_schema, c.table_name, c.column_name, c.data_type, c.is_nullable FROM information_schema.columns c JOIN information_schema.tables t ON t.table_schema = c.table_schema AND t.table_name = c.table_name WHERE c.table_schema NOT LIKE 'pg_%' AND c.table_schema <> 'information_schema' AND t.table_type IN ('BASE TABLE', 'VIEW') ORDER BY c.table_schema, c.table_name, c.ordinal_position";
