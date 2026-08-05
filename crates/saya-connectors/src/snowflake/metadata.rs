use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use serde_json::Value;

use super::{client::SnowflakeConnector, errors};
use crate::DatabaseConnector;

pub(crate) async fn schema(connector: &SnowflakeConnector) -> Result<SchemaTree, ConnectionError> {
    let database = connector
        .context
        .database
        .as_deref()
        .ok_or_else(errors::schema)?;
    let schema = connector
        .context
        .schema
        .as_deref()
        .ok_or_else(errors::schema)?;
    let sql = format!(
        "SELECT table_catalog, table_schema, table_name, column_name, data_type, is_nullable FROM {}.INFORMATION_SCHEMA.COLUMNS WHERE table_schema = '{}' AND table_schema <> 'INFORMATION_SCHEMA' ORDER BY table_name, ordinal_position",
        quote(database),
        literal(schema)
    );
    let output = connector
        .execute(saya_types::QueryRequest::new(sql, 10_000))
        .await
        .map_err(|_| errors::schema())?;
    let mut tables = BTreeMap::<String, Vec<Column>>::new();
    for row in output.rows {
        let values = row.as_array().ok_or_else(errors::schema)?;
        if values.len() != 6 {
            return Err(errors::schema());
        }
        let name = text(&values[2])?;
        tables.entry(name).or_default().push(Column {
            name: text(&values[3])?,
            data_type: text(&values[4])?,
            nullable: text(&values[5])?.eq_ignore_ascii_case("YES"),
        });
    }
    Ok(SchemaTree {
        databases: vec![Database {
            name: database.into(),
            schemas: vec![Schema {
                name: schema.into(),
                tables: tables
                    .into_iter()
                    .map(|(name, columns)| Table { name, columns })
                    .collect(),
            }],
        }],
    })
}

fn text(value: &Value) -> Result<String, ConnectionError> {
    value.as_str().map(str::to_owned).ok_or_else(errors::schema)
}
fn quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn literal(value: &str) -> String {
    value.replace('\'', "''")
}
