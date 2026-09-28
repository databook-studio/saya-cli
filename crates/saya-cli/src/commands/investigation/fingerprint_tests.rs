//! Unit tests for `fingerprint::combined_fingerprint`: how the definition's
//! referenced objects resolve against a live schema tree and fold into one
//! digest (invariant 2).

use super::combined_fingerprint;
use saya_types::{Column, Database, Schema, SchemaTree, Table};

/// A tree spec: `(database, [(schema, [tables])])`.
type TreeSpec<'a> = Vec<(&'a str, Vec<(&'a str, Vec<&'a str>)>)>;

fn tree(databases: TreeSpec<'_>) -> SchemaTree {
    SchemaTree {
        databases: databases
            .into_iter()
            .map(|(database, schemas)| Database {
                name: database.into(),
                schemas: schemas
                    .into_iter()
                    .map(|(schema, tables)| Schema {
                        name: schema.into(),
                        tables: tables
                            .into_iter()
                            .map(|name| Table {
                                name: name.into(),
                                columns: vec![Column {
                                    name: "id".into(),
                                    data_type: "bigint".into(),
                                    nullable: false,
                                }],
                                primary_key: vec![],
                                foreign_keys: vec![],
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// The production sqlite tree shape: database = the file's stem, one schema
/// named `main`.
fn sqlite_tree() -> SchemaTree {
    tree(vec![("data", vec![("main", vec!["events", "orders"])])])
}

#[test]
fn one_part_name_resolves_anywhere_in_the_tree() {
    let digest = combined_fingerprint(&sqlite_tree(), &["events".to_string()]);
    assert!(digest.is_some(), "sqlite names tables flatly: {digest:?}");
}

#[test]
fn two_part_name_resolves_by_schema_across_databases() {
    let postgres_tree = tree(vec![("analytics", vec![("public", vec!["events"])])]);
    let digest = combined_fingerprint(&postgres_tree, &["public.events".to_string()]);
    assert!(
        digest.is_some(),
        "schema-qualified name resolves: {digest:?}"
    );
}

#[test]
fn three_part_name_resolves_exactly() {
    let digest = combined_fingerprint(&postgres_tree(), &["analytics.public.events".to_string()]);
    assert!(
        digest.is_some(),
        "fully-qualified name resolves: {digest:?}"
    );
}

#[test]
fn wrong_qualifier_is_missing_not_found() {
    // The table exists, but not under the named schema: the entry records the
    // object as missing rather than silently resolving somewhere else.
    let missing = combined_fingerprint(&postgres_tree(), &["analytics.private.events".to_string()]);
    let resolved = combined_fingerprint(&postgres_tree(), &["analytics.public.events".to_string()]);
    assert_ne!(
        missing, resolved,
        "a missing object cannot impersonate a resolved one"
    );
}

fn postgres_tree() -> SchemaTree {
    tree(vec![("analytics", vec![("public", vec!["events"])])])
}

#[test]
fn zero_objects_yields_none() {
    assert_eq!(combined_fingerprint(&sqlite_tree(), &[]), None);
}

#[test]
fn fingerprint_is_order_independent_over_objects() {
    let sorted = combined_fingerprint(
        &sqlite_tree(),
        &["events".to_string(), "orders".to_string()],
    );
    let shuffled = combined_fingerprint(
        &sqlite_tree(),
        &["orders".to_string(), "events".to_string()],
    );
    assert_eq!(sorted, shuffled);
}

#[test]
fn fingerprint_changes_when_a_column_is_added() {
    let before = combined_fingerprint(&sqlite_tree(), &["events".to_string()]);
    let mut changed = sqlite_tree();
    changed.databases[0].schemas[0].tables[0]
        .columns
        .push(Column {
            name: "extra".into(),
            data_type: "text".into(),
            nullable: true,
        });
    let after = combined_fingerprint(&changed, &["events".to_string()]);
    assert_ne!(before, after, "an added column invalidates the review");
}

#[test]
fn missing_object_is_stable_but_differs_from_resolved() {
    let with_missing =
        combined_fingerprint(&sqlite_tree(), &["ghost".to_string(), "events".to_string()]);
    let resolved =
        combined_fingerprint(&sqlite_tree(), &["events".to_string(), "other".to_string()]);
    // Both contain one resolved and one missing entry, but the object names
    // differ, so the digests differ.
    assert_ne!(with_missing, resolved);
}
