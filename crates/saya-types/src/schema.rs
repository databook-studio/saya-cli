use serde::{Deserialize, Serialize};

/// A complete schema snapshot returned by a connector.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SchemaTree {
    pub databases: Vec<Database>,
}

impl SchemaTree {
    /// Find a table by its three-part name, case-insensitive on each part —
    /// the convention the schema fingerprint and the validity reconciler use,
    /// so a name that kept its casing but changed type still resolves here.
    /// Returns `None` when any part is absent. Pure lookup over the tree; the
    /// caller decides what absence means (refuse, mark stale, fall back).
    pub fn find_table(&self, catalog: &str, schema: &str, table: &str) -> Option<&Table> {
        self.databases
            .iter()
            .find(|db| db.name.eq_ignore_ascii_case(catalog))
            .and_then(|db| {
                db.schemas
                    .iter()
                    .find(|s| s.name.eq_ignore_ascii_case(schema))
            })
            .and_then(|s| s.tables.iter().find(|t| t.name.eq_ignore_ascii_case(table)))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Database {
    pub name: String,
    #[serde(default)]
    pub schemas: Vec<Schema>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Schema {
    pub name: String,
    #[serde(default)]
    pub tables: Vec<Table>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Table {
    pub name: String,
    #[serde(default)]
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree_with(table: &str) -> SchemaTree {
        SchemaTree {
            databases: vec![Database {
                name: "analytics".into(),
                schemas: vec![Schema {
                    name: "public".into(),
                    tables: vec![Table {
                        name: table.into(),
                        columns: vec![Column {
                            name: "id".into(),
                            data_type: "bigint".into(),
                            nullable: false,
                        }],
                    }],
                }],
            }],
        }
    }

    #[test]
    fn find_table_resolves_case_insensitively() {
        let tree = tree_with("orders");
        let found = tree
            .find_table("Analytics", "PUBLIC", "Orders")
            .expect("case-insensitive on every part");
        assert_eq!(found.name, "orders");
    }

    #[test]
    fn find_table_none_when_any_part_absent() {
        let tree = tree_with("orders");
        assert!(tree.find_table("analytics", "public", "ghost").is_none());
        assert!(tree.find_table("analytics", "private", "orders").is_none());
        assert!(tree.find_table("other", "public", "orders").is_none());
    }

    #[test]
    fn find_table_none_on_empty_tree() {
        let tree = SchemaTree::default();
        assert!(tree.find_table("analytics", "public", "orders").is_none());
    }
}
