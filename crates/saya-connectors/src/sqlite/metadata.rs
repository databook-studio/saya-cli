use std::collections::{HashMap, HashSet};

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use sqlx::Row;
use tokio::time::timeout;

use super::{SqliteConnector, errors};

const TABLE_LIST_SQL: &str = r#"SELECT name, wr FROM pragma_table_list WHERE schema = 'main'"#;

const PK_INDEX_SQL: &str = r#"SELECT DISTINCT s.name FROM sqlite_schema AS s, pragma_index_list(s.name) AS i WHERE s.type = 'table' AND i.origin = 'pk'"#;

const SCHEMA_SQL: &str = r#"SELECT s.name, x.name, x.type, x."notnull", x.pk FROM sqlite_schema AS s, pragma_table_xinfo(s.name, 'main') AS x WHERE s.type IN ('table','view') AND s.name NOT LIKE 'sqlite_%' ORDER BY s.name, x.cid"#;

struct RawColumn {
    name: String,
    declared_type: String,
    notnull: i64,
    pk: i64,
}

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
    let wr_rows = timeout(
        c.query_timeout,
        sqlx::query(TABLE_LIST_SQL).fetch_all(&c.pool),
    )
    .await
    .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))?
    .map_err(errors::schema)?;

    let mut wr_map: HashMap<String, bool> = HashMap::new();
    for row in wr_rows {
        let name: String = row.try_get(0).map_err(errors::row)?;
        let wr: i64 = row.try_get(1).map_err(errors::row)?;
        wr_map.insert(name, wr != 0);
    }

    let pk_index_rows = timeout(
        c.query_timeout,
        sqlx::query(PK_INDEX_SQL).fetch_all(&c.pool),
    )
    .await
    .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))?
    .map_err(errors::schema)?;

    let mut pk_index_set: HashSet<String> = HashSet::new();
    for row in pk_index_rows {
        let name: String = row.try_get(0).map_err(errors::row)?;
        pk_index_set.insert(name);
    }

    let rows = timeout(c.query_timeout, sqlx::query(SCHEMA_SQL).fetch_all(&c.pool))
        .await
        .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))?
        .map_err(errors::schema)?;

    let mut tables: Vec<Table> = Vec::new();
    let mut current_table_name: Option<String> = None;
    let mut current_raw_columns: Vec<RawColumn> = Vec::new();

    for row in rows {
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
                tables.push(build_table(
                    name,
                    std::mem::take(&mut current_raw_columns),
                    &wr_map,
                    &pk_index_set,
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
        tables.push(build_table(
            name,
            current_raw_columns,
            &wr_map,
            &pk_index_set,
        ));
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

fn build_table(
    name: String,
    raw_columns: Vec<RawColumn>,
    wr_map: &HashMap<String, bool>,
    pk_index_set: &HashSet<String>,
) -> Table {
    let without_rowid = wr_map.get(&name).copied().unwrap_or(false);
    let has_pk_index = pk_index_set.contains(&name);
    let pk_column_count = raw_columns.iter().filter(|c| c.pk > 0).count();

    let columns = raw_columns
        .into_iter()
        .map(|col| {
            let rowid_alias = !without_rowid
                && pk_column_count == 1
                && col.pk == 1
                && col.declared_type.eq_ignore_ascii_case("INTEGER")
                && !has_pk_index;
            let non_null = col.notnull != 0 || (without_rowid && col.pk >= 1) || rowid_alias;
            Column {
                name: col.name,
                data_type: col.declared_type,
                nullable: !non_null,
            }
        })
        .collect();

    Table { name, columns }
}
