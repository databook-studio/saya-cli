use std::collections::{HashMap, HashSet};

use futures_util::TryStreamExt;
use saya_types::{ConnectionError, Database, Schema, SchemaTree, Table};
use saya_types::{MAX_SCHEMA_COLUMNS, MAX_SCHEMA_TABLES};
use sqlx::Row;
use tokio::time::timeout;

use super::foreign_keys::load_foreign_keys;
use super::table::{RawColumn, build_table};
use crate::sqlite::{SqliteConnector, errors};

const TABLE_LIST_SQL: &str = r#"SELECT name, wr FROM pragma_table_list WHERE schema = 'main'"#;

const PK_INDEX_SQL: &str = r#"SELECT DISTINCT s.name FROM sqlite_schema AS s, pragma_index_list(s.name) AS i WHERE s.type = 'table' AND i.origin = 'pk'"#;

const SCHEMA_SQL: &str = r#"SELECT s.name, x.name, x.type, x."notnull", x.pk FROM sqlite_schema AS s, pragma_table_xinfo(s.name, 'main') AS x WHERE s.type IN ('table','view') AND s.name NOT LIKE 'sqlite_%' ORDER BY s.name, x.cid"#;

/// Inspects SQLite database schema and returns a [`SchemaTree`].
///
/// ### Authoritative Nullability Contract
/// A column is NON-nullable iff any of:
/// - **(a)** declared `NOT NULL` (`notnull != 0`), OR
/// - **(b)** it is the `INTEGER PRIMARY KEY` rowid alias — the table is a rowid table (`pragma_table_list.wr == 0`)
///   AND the `PRIMARY KEY` is a SINGLE column AND that column's declared type is exactly `"INTEGER"`
///   (case-insensitive) AND the table has NO auto-index of origin `'pk'` (this excludes `INTEGER PRIMARY KEY DESC`,
///   which SQLite backs with a real index and does NOT make a rowid alias), OR
/// - **(c)** it is part of the `PRIMARY KEY` of a `WITHOUT ROWID` table (`wr == 1` AND `pk >= 1`), where SQLite enforces `NOT NULL`.
///
/// Otherwise the column is nullable. Views follow (a) only.
pub(crate) async fn schema(c: &SqliteConnector) -> Result<SchemaTree, ConnectionError> {
    let wr_map = timeout(c.query_timeout, async {
        let mut stream = sqlx::query(TABLE_LIST_SQL).fetch(&c.pool);
        let mut map = HashMap::new();
        while let Some(row) = stream.try_next().await.map_err(errors::schema)? {
            if map.len() == MAX_SCHEMA_TABLES {
                return Err(ConnectionError::schema_failed(
                    "SQLite schema has too many tables",
                ));
            }
            let name: String = row.try_get(0).map_err(errors::row)?;
            let wr: i64 = row.try_get(1).map_err(errors::row)?;
            map.insert(name, wr != 0);
        }
        Ok(map)
    })
    .await
    .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))??;

    let pk_index_set = timeout(c.query_timeout, async {
        let mut stream = sqlx::query(PK_INDEX_SQL).fetch(&c.pool);
        let mut set = HashSet::new();
        while let Some(row) = stream.try_next().await.map_err(errors::schema)? {
            if set.len() == MAX_SCHEMA_TABLES {
                return Err(ConnectionError::schema_failed(
                    "SQLite schema has too many tables",
                ));
            }
            let name: String = row.try_get(0).map_err(errors::row)?;
            set.insert(name);
        }
        Ok(set)
    })
    .await
    .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))??;

    let mut fk_map = load_foreign_keys(c).await?;

    let tables = timeout(c.query_timeout, async {
        let mut stream = sqlx::query(SCHEMA_SQL).fetch(&c.pool);
        let mut tables: Vec<Table> = Vec::new();
        let mut current_table_name: Option<String> = None;
        let mut current_raw_columns: Vec<RawColumn> = Vec::new();
        let mut columns_seen = 0;

        while let Some(row) = stream.try_next().await.map_err(errors::schema)? {
            if columns_seen == MAX_SCHEMA_COLUMNS {
                return Err(ConnectionError::schema_failed(
                    "SQLite schema has too many columns",
                ));
            }
            columns_seen += 1;
            let table_name: String = row.try_get(0).map_err(errors::row)?;
            let col = RawColumn {
                name: row.try_get(1).map_err(errors::row)?,
                declared_type: row.try_get(2).map_err(errors::row)?,
                notnull: row.try_get(3).map_err(errors::row)?,
                pk: row.try_get(4).map_err(errors::row)?,
            };

            match current_table_name {
                Some(ref name) if name == &table_name => {
                    current_raw_columns.push(col);
                }
                Some(name) => {
                    if tables.len() == MAX_SCHEMA_TABLES {
                        return Err(ConnectionError::schema_failed(
                            "SQLite schema has too many tables",
                        ));
                    }
                    let foreign_keys = fk_map.remove(&name).unwrap_or_default();
                    tables.push(build_table(
                        name,
                        std::mem::take(&mut current_raw_columns),
                        &wr_map,
                        &pk_index_set,
                        foreign_keys,
                    ));
                    current_table_name = Some(table_name);
                    current_raw_columns.push(col);
                }
                None => {
                    current_table_name = Some(table_name);
                    current_raw_columns.push(col);
                }
            }
        }

        if let Some(name) = current_table_name {
            if tables.len() == MAX_SCHEMA_TABLES {
                return Err(ConnectionError::schema_failed(
                    "SQLite schema has too many tables",
                ));
            }
            let foreign_keys = fk_map.remove(&name).unwrap_or_default();
            tables.push(build_table(
                name,
                current_raw_columns,
                &wr_map,
                &pk_index_set,
                foreign_keys,
            ));
        }
        Ok(tables)
    })
    .await
    .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))??;

    let tree = SchemaTree {
        databases: vec![Database {
            name: c.database.clone(),
            schemas: vec![Schema {
                name: "main".into(),
                tables,
            }],
        }],
    };
    tree.validate()
        .map_err(|_| ConnectionError::schema_failed("SQLite schema exceeds configured limits"))?;
    Ok(tree)
}
