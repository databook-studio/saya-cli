use std::collections::BTreeMap;

use saya_types::{Column, ConnectionError, Database, Schema, SchemaTree, Table};
use serde_json::Value;
use tokio::time::timeout;

use super::{ClickHouseConnector, errors};

/// Bound on the schema query: one million column rows is far beyond a real
/// deployment yet still finite, so a runaway catalog fails closed instead of
/// growing without limit. The `LIMIT` in [`SCHEMA_SQL`] caps the result
/// gracefully; this number is the server-side backstop.
const SCHEMA_CAP: usize = 1_000_000;

/// One round trip over the catalog. `system.columns` joined to `system.tables`
/// yields every column of every user table in declared order; system databases
/// are excluded so only the user's own databases appear. The connector sends
/// this directly (it is not caller SQL) so it bypasses the safety layer.
const SCHEMA_SQL: &str = "SELECT t.database AS db, t.name AS tbl, c.name AS col, c.type AS typ \
FROM system.columns c \
INNER JOIN system.tables t ON t.database = c.database AND t.name = c.table \
WHERE t.database NOT IN ('system', 'INFORMATION_SCHEMA', 'information_schema') \
ORDER BY t.database, t.name, c.position \
LIMIT 1000000";

pub(crate) async fn schema(connector: &ClickHouseConnector) -> Result<SchemaTree, ConnectionError> {
    let response = timeout(
        connector.query_timeout,
        connector.build_request(SCHEMA_SQL, SCHEMA_CAP).send(),
    )
    .await
    .map_err(|_| errors::schema_timeout())?
    .map_err(|_| errors::schema())?;
    if !response.status().is_success() {
        return Err(errors::schema());
    }
    let value: Value = response.json().await.map_err(|_| errors::schema())?;
    let rows = value
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        entries.push((
            row.get("db")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            row.get("tbl")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            row.get("col")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            row.get("typ")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        ));
    }
    Ok(build_tree(entries))
}

/// Builds the schema tree from catalog rows. ClickHouse has databases and
/// tables but no schema layer and no foreign keys, so each database becomes one
/// schema under a single `ClickHouse` database entry — the same shape the MySQL
/// connector uses for its flat `database.table` namespace — and every table
/// carries an empty foreign-key list rather than an invented one.
fn build_tree(rows: Vec<(String, String, String, String)>) -> SchemaTree {
    let mut databases: BTreeMap<String, BTreeMap<String, Vec<Column>>> = BTreeMap::new();
    for (database, table, column, data_type) in rows {
        // ClickHouse encodes nullability in the type itself (`Nullable(...)`):
        // `system.columns` has no separate flag, so the type string is the only
        // source. This is a display-time heuristic, not a null-safety contract.
        let nullable = data_type.contains("Nullable");
        databases
            .entry(database)
            .or_default()
            .entry(table)
            .or_default()
            .push(Column {
                name: column,
                data_type,
                nullable,
            });
    }

    let schemas = databases
        .into_iter()
        .map(|(name, tables)| {
            let built_tables = tables
                .into_iter()
                .map(|(table_name, columns)| Table {
                    name: table_name,
                    columns,
                    primary_key: vec![],
                    // ClickHouse has no foreign keys; none are synthesized.
                    foreign_keys: vec![],
                })
                .collect();
            Schema {
                name,
                tables: built_tables,
            }
        })
        .collect();
    SchemaTree {
        databases: vec![Database {
            name: "ClickHouse".into(),
            schemas,
        }],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saya_types::ForeignKey;

    fn table_in<'a>(schema: &'a Schema, name: &str) -> &'a Table {
        schema
            .tables
            .iter()
            .find(|table| table.name == name)
            .unwrap_or_else(|| panic!("{name} table missing"))
    }

    #[test]
    fn build_tree_groups_columns_by_database_and_table() {
        let rows = vec![
            (
                "analytics".into(),
                "orders".into(),
                "id".into(),
                "UInt64".into(),
            ),
            (
                "analytics".into(),
                "orders".into(),
                "amount".into(),
                "Nullable(Decimal(18,2))".into(),
            ),
            (
                "analytics".into(),
                "customers".into(),
                "email".into(),
                "String".into(),
            ),
            (
                "logs".into(),
                "events".into(),
                "ts".into(),
                "DateTime".into(),
            ),
        ];
        let tree = build_tree(rows);
        assert_eq!(tree.databases.len(), 1);
        assert_eq!(tree.databases[0].name, "ClickHouse");
        // Databases are keyed in a BTreeMap, so they surface in sorted order.
        let schemas = &tree.databases[0].schemas;
        assert_eq!(schemas.len(), 2);
        assert_eq!(schemas[0].name, "analytics");
        assert_eq!(schemas[1].name, "logs");

        let orders = table_in(&schemas[0], "orders");
        assert_eq!(orders.columns.len(), 2);
        assert_eq!(orders.columns[0].name, "id");
        assert!(!orders.columns[0].nullable);
        assert_eq!(orders.columns[1].name, "amount");
        assert!(orders.columns[1].nullable);
        let customers = table_in(&schemas[0], "customers");
        assert_eq!(customers.columns.len(), 1);
        assert_eq!(customers.columns[0].name, "email");
        let events = table_in(&schemas[1], "events");
        assert_eq!(events.columns[0].name, "ts");
        assert!(!events.columns[0].nullable);
    }

    #[test]
    fn build_tree_returns_no_foreign_keys() {
        let rows = vec![("db".into(), "t".into(), "c".into(), "Int32".into())];
        let tree = build_tree(rows);
        for database in &tree.databases {
            for schema in &database.schemas {
                for table in &schema.tables {
                    assert!(table.foreign_keys.is_empty());
                    assert!(table.primary_key.is_empty());
                }
            }
        }
        // The empty list is a real value, not a default that a caller might
        // overlook: ForeignKey is never constructed for ClickHouse.
        assert_eq!(
            tree.databases[0].schemas[0].tables[0].foreign_keys,
            Vec::<ForeignKey>::new()
        );
    }

    #[test]
    fn build_tree_with_no_rows_is_still_well_formed() {
        let tree = build_tree(Vec::new());
        assert_eq!(tree.databases.len(), 1);
        assert_eq!(tree.databases[0].name, "ClickHouse");
        assert!(tree.databases[0].schemas.is_empty());
    }
}
