use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{SqliteConnector, errors};

const SCHEMA_SQL: &str = r#"SELECT s.name, x.name, x.type, x."notnull", x.pk, s.sql FROM sqlite_schema AS s, pragma_table_xinfo(s.name, 'main') AS x WHERE s.type IN ('table','view') AND s.name NOT LIKE 'sqlite_%' ORDER BY s.name, x.cid"#;

/// Inspects SQLite database schema and returns a [`SchemaTree`].
///
/// ### Nullability Contract
/// A column is non-nullable iff any of:
/// - **(a)** it is declared `NOT NULL` (`notnull != 0`), OR
/// - **(b)** it is the `INTEGER PRIMARY KEY` rowid alias — declared type is exactly `"INTEGER"` (case-insensitive)
///   AND `pk == 1` AND the table is a rowid table (not `WITHOUT ROWID`), OR
/// - **(c)** it is part of the `PRIMARY KEY` of a `WITHOUT ROWID` table (`pk >= 1`), where SQLite enforces `NOT NULL`.
///
/// Otherwise the column is nullable.
pub(crate) async fn schema(c: &SqliteConnector) -> Result<SchemaTree, ConnectionError> {
    let rows = timeout(c.query_timeout, sqlx::query(SCHEMA_SQL).fetch_all(&c.pool))
        .await
        .map_err(|_| ConnectionError::SchemaFailed("SQLite schema discovery timed out".into()))?
        .map_err(errors::schema)?;

    let mut tables: Vec<Table> = Vec::new();
    let mut current_table_name: Option<String> = None;
    let mut current_columns: Vec<Column> = Vec::new();

    for row in rows {
        let table_name: String = row.try_get(0).map_err(errors::row)?;
        let column_name: String = row.try_get(1).map_err(errors::row)?;
        let declared_type: String = row.try_get(2).map_err(errors::row)?;
        let notnull: i64 = row.try_get(3).map_err(errors::row)?;
        let pk: i64 = row.try_get(4).map_err(errors::row)?;
        let table_sql: Option<String> = row.try_get(5).map_err(errors::row)?;

        let without_rowid = table_sql
            .as_deref()
            .map(|sql| sql.to_uppercase().contains("WITHOUT ROWID"))
            .unwrap_or(false);

        let integer_pk_rowid_alias =
            pk == 1 && declared_type.eq_ignore_ascii_case("INTEGER") && !without_rowid;
        let non_null = notnull != 0 || integer_pk_rowid_alias || (without_rowid && pk >= 1);
        let nullable = !non_null;

        let column = Column {
            name: column_name,
            data_type: declared_type,
            nullable,
        };

        match current_table_name {
            Some(ref name) if name == &table_name => {
                current_columns.push(column);
            }
            Some(name) => {
                tables.push(Table {
                    name,
                    columns: std::mem::take(&mut current_columns),
                });
                current_table_name = Some(table_name);
                current_columns.push(column);
            }
            None => {
                current_table_name = Some(table_name);
                current_columns.push(column);
            }
        }
    }

    if let Some(name) = current_table_name {
        tables.push(Table {
            name,
            columns: current_columns,
        });
    }

    Ok(SchemaTree {
        databases: vec![Database {
            name: c.database.clone(),
            schemas: vec![Schema {
                name: "main".into(),
                tables,
            }],
        }],
    })
}
