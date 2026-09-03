use std::collections::{BTreeMap, HashMap};

use futures_util::TryStreamExt;
use saya_types::{Column, ConnectionError, Database, ForeignKey, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{MySqlConnector, errors};

/// Columns of one foreign-key constraint as its rows arrive: the local
/// columns, the referenced schema and table, and the referenced columns,
/// keyed by the table and constraint the rows belong to. MySQL carries the
/// referenced schema because a constraint may point outside its own.
type ConstraintRows = BTreeMap<(String, String), (Vec<String>, String, String, Vec<String>)>;

const SCHEMA_SQL: &str = "SELECT CAST(TABLE_NAME AS CHAR) AS table_name, CAST(COLUMN_NAME AS CHAR) AS column_name, CAST(DATA_TYPE AS CHAR) AS data_type, CAST(IS_NULLABLE AS CHAR) AS is_nullable FROM information_schema.columns WHERE TABLE_SCHEMA = ? ORDER BY TABLE_NAME, ORDINAL_POSITION";

/// `key_column_usage` carries the referenced side on the same rows as the
/// referencing side, so a single filtered query returns every foreign key in
/// the database. `REFERENCED_TABLE_NAME IS NOT NULL` keeps only FK rows;
/// `ORDINAL_POSITION` orders a composite key's columns so they pair up.
const FK_SQL: &str = "SELECT CAST(TABLE_NAME AS CHAR) AS table_name, CAST(COLUMN_NAME AS CHAR) AS column_name, CAST(CONSTRAINT_NAME AS CHAR) AS constraint_name, CAST(REFERENCED_TABLE_SCHEMA AS CHAR) AS ref_schema, CAST(REFERENCED_TABLE_NAME AS CHAR) AS ref_table, CAST(REFERENCED_COLUMN_NAME AS CHAR) AS ref_column FROM information_schema.key_column_usage WHERE TABLE_SCHEMA = ? AND REFERENCED_TABLE_NAME IS NOT NULL ORDER BY TABLE_NAME, CONSTRAINT_NAME, ORDINAL_POSITION";

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
    let mut foreign_keys = load_foreign_keys(connector).await?;

    let mut built_tables: Vec<Table> = Vec::new();
    for (name, columns) in tables {
        let foreign_keys = foreign_keys.remove(&name).unwrap_or_default();
        built_tables.push(Table {
            name,
            columns,
            primary_key: vec![],
            foreign_keys,
        });
    }
    Ok(SchemaTree {
        databases: vec![Database {
            name: "MySQL".into(),
            schemas: vec![Schema {
                name: connector.database.clone(),
                tables: built_tables,
            }],
        }],
    })
}

async fn load_foreign_keys(
    connector: &MySqlConnector,
) -> Result<HashMap<String, Vec<ForeignKey>>, ConnectionError> {
    let rows = timeout(
        connector.query_timeout,
        sqlx::query(FK_SQL)
            .bind(&connector.database)
            .fetch_all(&connector.pool),
    )
    .await
    .map_err(|_| ConnectionError::schema_failed("MySQL schema discovery timed out"))?
    .map_err(errors::schema)?;

    let mut by_constraint: ConstraintRows = BTreeMap::new();
    for row in rows {
        let table: String = row.try_get("table_name").map_err(errors::schema)?;
        let constraint: String = row.try_get("constraint_name").map_err(errors::schema)?;
        let column: String = row.try_get("column_name").map_err(errors::schema)?;
        let ref_schema: String = row.try_get("ref_schema").map_err(errors::schema)?;
        let ref_table: String = row.try_get("ref_table").map_err(errors::schema)?;
        let ref_column: String = row.try_get("ref_column").map_err(errors::schema)?;
        let entry = by_constraint
            .entry((table, constraint))
            .or_insert_with(|| (Vec::new(), ref_schema, ref_table, Vec::new()));
        entry.0.push(column);
        entry.3.push(ref_column);
    }

    let mut map: HashMap<String, Vec<ForeignKey>> = HashMap::new();
    for ((table, _), (columns, ref_schema, ref_table, ref_columns)) in by_constraint {
        map.entry(table).or_default().push(ForeignKey {
            columns,
            referenced_schema: Some(ref_schema),
            referenced_table: ref_table,
            referenced_columns: ref_columns,
        });
    }
    Ok(map)
}
