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
    /// Columns forming the primary key, in key order. Empty means the
    /// connector did not discover one — SQLite fills this from the primary-key
    /// flags it already reads; the other connectors leave it empty until they
    /// gain extraction. Kept on the table rather than per-column so an absent
    /// key reads as "unknown", not as a per-column "definitely not" claim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub primary_key: Vec<String>,
    /// Foreign-key constraints on this table. Empty when the connector found
    /// none or does not extract them for that dialect.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub foreign_keys: Vec<ForeignKey>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Column {
    pub name: String,
    pub data_type: String,
    pub nullable: bool,
}

/// A foreign-key constraint: the local columns that reference another table
/// and the columns they point at. Composite keys carry more than one column
/// in each vector, paired positionally — the first referencing column points
/// at the first referenced column, and so on.
///
/// `referenced_schema` is set only by connectors whose dialect names a schema
/// for the target (PostgreSQL, MySQL); SQLite leaves it `None` because its
/// foreign keys always resolve within the same database. A connector that
/// cannot extract foreign keys produces no `ForeignKey` rows at all rather
/// than partial ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKey {
    /// The referencing (local) columns, in declaration order.
    pub columns: Vec<String>,
    /// Schema of the referenced table when the dialect names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub referenced_schema: Option<String>,
    /// The referenced table.
    pub referenced_table: String,
    /// The referenced columns, positionally paired with `columns`.
    pub referenced_columns: Vec<String>,
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
                        primary_key: vec!["id".into()],
                        foreign_keys: vec![],
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

    /// Schemas cached by an older build were written before these fields
    /// existed. They must still load, with the missing keys reading as empty
    /// rather than failing the whole cache — otherwise a user upgrades and the
    /// first schema refresh after a live failure can no longer fall back.
    #[test]
    fn table_without_key_fields_deserializes_from_old_cache() {
        let old_cache = r#"{
            "name": "orders",
            "columns": [
                {"name": "id", "data_type": "bigint", "nullable": false}
            ]
        }"#;
        let table: Table = serde_json::from_str(old_cache).expect("old cached table loads");
        assert_eq!(table.name, "orders");
        assert!(table.primary_key.is_empty());
        assert!(table.foreign_keys.is_empty());
    }

    /// Empty key fields are omitted on write so a table with no keys serializes
    /// exactly as it did before the fields existed — the cached bytes stay
    /// stable and no downstream snapshot of the tree sees a spurious diff.
    #[test]
    fn empty_key_fields_are_omitted_when_serialized() {
        let table = Table {
            name: "orders".into(),
            columns: vec![Column {
                name: "id".into(),
                data_type: "bigint".into(),
                nullable: false,
            }],
            primary_key: vec![],
            foreign_keys: vec![],
        };
        let json = serde_json::to_string(&table).unwrap();
        assert!(!json.contains("primary_key"));
        assert!(!json.contains("foreign_keys"));
    }

    /// A composite, self-referencing constraint must round-trip with its
    /// column order intact: positional pairing is the only thing that keeps
    /// `(parent_a, parent_b) -> (a, b)` from being read as `(a, b) -> (b, a)`.
    #[test]
    fn composite_foreign_key_round_trips_with_column_order() {
        let fk = ForeignKey {
            columns: vec!["parent_id".into(), "parent_seq".into()],
            referenced_schema: None,
            referenced_table: "nodes".into(),
            referenced_columns: vec!["id".into(), "seq".into()],
        };
        let table = Table {
            name: "nodes".into(),
            columns: vec![],
            primary_key: vec!["id".into(), "seq".into()],
            foreign_keys: vec![fk.clone()],
        };
        let json = serde_json::to_string(&table).unwrap();
        let back: Table = serde_json::from_str(&json).unwrap();
        assert_eq!(back.primary_key, vec!["id".to_string(), "seq".to_string()]);
        assert_eq!(back.foreign_keys.len(), 1);
        assert_eq!(back.foreign_keys[0], fk);
    }

    /// A cross-schema reference carries the target schema so a reader can
    /// resolve it unambiguously when two schemas hold a table of the same name.
    #[test]
    fn foreign_key_keeps_referenced_schema_when_present() {
        let fk = ForeignKey {
            columns: vec!["user_id".into()],
            referenced_schema: Some("auth".into()),
            referenced_table: "users".into(),
            referenced_columns: vec!["id".into()],
        };
        let json = serde_json::to_string(&fk).unwrap();
        let back: ForeignKey = serde_json::from_str(&json).unwrap();
        assert_eq!(back.referenced_schema.as_deref(), Some("auth"));
    }
}
