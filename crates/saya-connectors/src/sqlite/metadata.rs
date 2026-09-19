use std::collections::{BTreeMap, HashMap, HashSet};

use futures_util::TryStreamExt;
use saya_types::{Column, ConnectionError, Database, ForeignKey, Schema, SchemaTree, Table};
use saya_types::{MAX_SCHEMA_COLUMNS, MAX_SCHEMA_TABLES};
use sqlx::Row;
use tokio::time::timeout;

use super::{SqliteConnector, errors};

/// Foreign-key rows grouped per table, then per constraint id as
/// `PRAGMA foreign_key_list` reports it: the local columns, the referenced
/// table, and the referenced columns.
type TableConstraints = HashMap<String, BTreeMap<i64, (Vec<String>, String, Vec<String>)>>;

const TABLE_LIST_SQL: &str = r#"SELECT name, wr FROM pragma_table_list WHERE schema = 'main'"#;

const PK_INDEX_SQL: &str = r#"SELECT DISTINCT s.name FROM sqlite_schema AS s, pragma_index_list(s.name) AS i WHERE s.type = 'table' AND i.origin = 'pk'"#;

const SCHEMA_SQL: &str = r#"SELECT s.name, x.name, x.type, x."notnull", x.pk FROM sqlite_schema AS s, pragma_table_xinfo(s.name, 'main') AS x WHERE s.type IN ('table','view') AND s.name NOT LIKE 'sqlite_%' ORDER BY s.name, x.cid"#;

const FK_SQL: &str = r#"SELECT s.name AS tbl, f.id AS fk_id, f.seq AS fk_seq, f."table" AS ref_tbl, f."from" AS from_col, f."to" AS to_col FROM sqlite_schema AS s, pragma_foreign_key_list(s.name) AS f WHERE s.type = 'table' AND s.name NOT LIKE 'sqlite_%' ORDER BY s.name, f.id, f.seq"#;

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

fn build_table(
    name: String,
    raw_columns: Vec<RawColumn>,
    wr_map: &HashMap<String, bool>,
    pk_index_set: &HashSet<String>,
    foreign_keys: Vec<ForeignKey>,
) -> Table {
    let without_rowid = wr_map.get(&name).copied().unwrap_or(false);
    let has_pk_index = pk_index_set.contains(&name);
    let pk_column_count = raw_columns.iter().filter(|c| c.pk > 0).count();

    // The pragma reports primary-key membership as a 1-based ordinal on each
    // column, so sorting by it preserves a composite key's column order —
    // the order a writer needs to match when joining on the whole key.
    let mut primary_key: Vec<(i64, String)> = raw_columns
        .iter()
        .filter(|c| c.pk > 0)
        .map(|c| (c.pk, c.name.clone()))
        .collect();
    primary_key.sort_by_key(|(ord, _)| *ord);
    let primary_key: Vec<String> = primary_key.into_iter().map(|(_, name)| name).collect();

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

    Table {
        name,
        columns,
        primary_key,
        foreign_keys,
    }
}

/// One round trip returning every foreign key on every table, rather than a
/// query per table — a wide schema would otherwise pay one round trip per
/// table and trip the discovery timeout. `pragma_foreign_key_list` exposed as
/// a table-valued function joins against `sqlite_schema` to do that in a
/// single statement; `id` groups a (possibly composite) constraint and `seq`
/// orders the columns within it.
async fn load_foreign_keys(
    c: &SqliteConnector,
) -> Result<HashMap<String, Vec<ForeignKey>>, ConnectionError> {
    let by_table = timeout(c.query_timeout, async {
        let mut stream = sqlx::query(FK_SQL).fetch(&c.pool);
        let mut by_table: TableConstraints = HashMap::new();
        let mut rows_seen = 0;
        while let Some(row) = stream.try_next().await.map_err(errors::schema)? {
            if rows_seen == MAX_SCHEMA_COLUMNS {
                return Err(ConnectionError::schema_failed(
                    "SQLite schema has too many foreign-key columns",
                ));
            }
            rows_seen += 1;
            let table: String = row.try_get("tbl").map_err(errors::row)?;
            let fk_id: i64 = row.try_get("fk_id").map_err(errors::row)?;
            let referenced_table: String = row.try_get("ref_tbl").map_err(errors::row)?;
            let from_col: String = row.try_get("from_col").map_err(errors::row)?;
            let to_col: String = row.try_get("to_col").map_err(errors::row)?;
            let entry = by_table
                .entry(table)
                .or_default()
                .entry(fk_id)
                .or_insert_with(|| (Vec::new(), referenced_table, Vec::new()));
            entry.0.push(from_col);
            entry.2.push(to_col);
        }
        Ok(by_table)
    })
    .await
    .map_err(|_| ConnectionError::schema_failed("SQLite schema discovery timed out"))??;

    let mut map: HashMap<String, Vec<ForeignKey>> = HashMap::new();
    for (table, by_id) in by_table {
        let foreign_keys: Vec<ForeignKey> = by_id
            .into_values()
            .map(
                |(columns, referenced_table, referenced_columns)| ForeignKey {
                    columns,
                    referenced_schema: None,
                    referenced_table,
                    referenced_columns,
                },
            )
            .collect();
        map.insert(table, foreign_keys);
    }
    Ok(map)
}
