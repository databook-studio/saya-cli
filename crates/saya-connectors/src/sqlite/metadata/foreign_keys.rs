use std::collections::{BTreeMap, HashMap};

use futures_util::TryStreamExt;
use saya_types::{ConnectionError, ForeignKey, MAX_SCHEMA_COLUMNS};
use sqlx::Row;
use tokio::time::timeout;

use crate::sqlite::{SqliteConnector, errors};

/// Foreign-key rows grouped per table, then per constraint id as
/// `PRAGMA foreign_key_list` reports it: the local columns, the referenced
/// table, and the referenced columns.
type TableConstraints = HashMap<String, BTreeMap<i64, (Vec<String>, String, Vec<String>)>>;

const FK_SQL: &str = r#"SELECT s.name AS tbl, f.id AS fk_id, f.seq AS fk_seq, f."table" AS ref_tbl, f."from" AS from_col, f."to" AS to_col FROM sqlite_schema AS s, pragma_foreign_key_list(s.name) AS f WHERE s.type = 'table' AND s.name NOT LIKE 'sqlite_%' ORDER BY s.name, f.id, f.seq"#;

/// One round trip returning every foreign key on every table, rather than a
/// query per table — a wide schema would otherwise pay one round trip per
/// table and trip the discovery timeout. `pragma_foreign_key_list` exposed as
/// a table-valued function joins against `sqlite_schema` to do that in a
/// single statement; `id` groups a (possibly composite) constraint and `seq`
/// orders the columns within it.
pub(super) async fn load_foreign_keys(
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
