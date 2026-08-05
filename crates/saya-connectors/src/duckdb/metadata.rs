use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};

use super::{
    DuckDbConnector,
    execute::{self, Operation},
};

const SCHEMA_SQL: &str = "SELECT table_catalog, table_schema, table_name, column_name, data_type, is_nullable FROM information_schema.columns WHERE table_schema NOT IN ('information_schema', 'pg_catalog') ORDER BY table_catalog, table_schema, table_name, ordinal_position";

pub(crate) async fn schema(connector: &DuckDbConnector) -> Result<SchemaTree, ConnectionError> {
    let tree = execute::run(connector, Operation::Schema, |connection| {
        let mut statement = connection.prepare(SCHEMA_SQL).map_err(error)?;
        let mut rows = statement.query([]).map_err(error)?;
        let mut databases =
            BTreeMap::<String, BTreeMap<String, BTreeMap<String, Vec<Column>>>>::new();
        while let Some(row) = rows.next().map_err(error)? {
            let database: String = row.get(0).map_err(error)?;
            let schema: String = row.get(1).map_err(error)?;
            let table: String = row.get(2).map_err(error)?;
            let column = Column {
                name: row.get(3).map_err(error)?,
                data_type: row.get(4).map_err(error)?,
                nullable: row.get::<_, String>(5).map_err(error)? == "YES",
            };
            databases
                .entry(database)
                .or_default()
                .entry(schema)
                .or_default()
                .entry(table)
                .or_default()
                .push(column);
        }
        Ok(databases
            .into_iter()
            .map(|(name, schemas)| Database {
                name,
                schemas: schemas
                    .into_iter()
                    .map(|(name, tables)| Schema {
                        name,
                        tables: tables
                            .into_iter()
                            .map(|(name, columns)| Table { name, columns })
                            .collect(),
                    })
                    .collect(),
            })
            .collect())
    })
    .await?;
    Ok(SchemaTree { databases: tree })
}

fn error(_: duckdb::Error) -> ConnectionError {
    ConnectionError::SchemaFailed("DuckDB schema discovery failed".into())
}
