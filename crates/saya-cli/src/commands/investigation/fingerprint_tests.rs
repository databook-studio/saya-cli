//! Unit tests for the schema review (`fingerprint::analyze`, A2): the
//! definition's SQL yields the referenced parts, each resolves against the
//! live schema tree as exactly one table, missing or ambiguous resolution
//! makes the whole review unverifiable, and a complete review folds the
//! resolved per-table digests into one combined value.

use super::{Analysis, analyze, needs_schema};
use saya_connectors::SqlReferences;
use saya_types::{Column, Database, Schema, SchemaTree, Table};

/// A tree spec: `(database, [(schema, [tables])])`.
type TreeSpec<'a> = Vec<(&'a str, Vec<(&'a str, Vec<&'a str>)>)>;

fn tree(databases: TreeSpec<'_>) -> SchemaTree {
    tree_of_tables(
        databases
            .into_iter()
            .map(|(database, schemas)| {
                (
                    database,
                    schemas
                        .into_iter()
                        .map(|(schema, tables)| {
                            (
                                schema,
                                tables.iter().map(|name| table(name, &["id"])).collect(),
                            )
                        })
                        .collect(),
                )
            })
            .collect(),
    )
}

/// A tree spec over fully built tables: `(database, [(schema, [tables])])`.
type TableTreeSpec<'a> = Vec<(&'a str, Vec<(&'a str, Vec<Table>)>)>;

fn tree_of_tables(databases: TableTreeSpec<'_>) -> SchemaTree {
    SchemaTree {
        databases: databases
            .into_iter()
            .map(|(database, schemas)| Database {
                name: database.into(),
                schemas: schemas
                    .into_iter()
                    .map(|(schema, tables)| Schema {
                        name: schema.into(),
                        tables,
                    })
                    .collect(),
            })
            .collect(),
    }
}

fn table(name: &str, columns: &[&str]) -> Table {
    Table {
        name: name.into(),
        columns: columns
            .iter()
            .map(|column| Column {
                name: (*column).into(),
                data_type: "bigint".into(),
                nullable: false,
            })
            .collect(),
        primary_key: vec![],
        foreign_keys: vec![],
    }
}

/// References as `sql_references` reports them: parts as written, complete.
fn references(objects: &[&[&str]]) -> SqlReferences {
    SqlReferences {
        objects: objects
            .iter()
            .map(|parts| parts.iter().map(|part| (*part).to_string()).collect())
            .collect(),
        columns: Vec::new(),
        partial: false,
    }
}

fn partial_references(objects: &[&[&str]]) -> SqlReferences {
    SqlReferences {
        partial: true,
        ..references(objects)
    }
}

/// The production sqlite tree shape: database = the file's stem, one schema
/// named `main`.
fn sqlite_tree() -> SchemaTree {
    tree(vec![("data", vec![("main", vec!["events", "orders"])])])
}

fn complete(analysis: Analysis) -> Option<String> {
    match analysis {
        Analysis::Complete(fingerprint) => fingerprint,
        Analysis::Unverifiable(reason) => panic!("the review must complete, got: {reason}"),
    }
}

fn unverifiable(analysis: Analysis) -> String {
    match analysis {
        Analysis::Complete(fingerprint) => {
            panic!("the review must be unverifiable, got: {fingerprint:?}")
        }
        Analysis::Unverifiable(reason) => reason,
    }
}

#[test]
fn one_part_name_resolves_anywhere_in_the_tree() {
    let analysis = analyze(Some(&sqlite_tree()), Some(&references(&[&["events"]])));
    assert!(
        complete(analysis).is_some(),
        "sqlite names tables flatly, so a complete review binds a fingerprint"
    );
}

#[test]
fn two_part_name_resolves_by_schema_across_databases() {
    let postgres_tree = tree(vec![("analytics", vec![("public", vec!["events"])])]);
    let analysis = analyze(
        Some(&postgres_tree),
        Some(&references(&[&["public", "events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "schema-qualified name resolves"
    );
}

#[test]
fn three_part_name_resolves_exactly() {
    let analysis = analyze(
        Some(&postgres_tree()),
        Some(&references(&[&["analytics", "public", "events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "fully-qualified name resolves"
    );
}

#[test]
fn wrong_qualifier_is_missing_not_found() {
    // The table exists, but not under the named schema: the review is
    // unverifiable, never silently resolved somewhere else.
    let analysis = analyze(
        Some(&postgres_tree()),
        Some(&references(&[&["analytics", "private", "events"]])),
    );
    let reason = unverifiable(analysis);
    assert!(
        reason.contains(r#"table "analytics.private.events" not found"#),
        "{reason}"
    );
}

fn postgres_tree() -> SchemaTree {
    tree(vec![("analytics", vec![("public", vec!["events"])])])
}

#[test]
fn zero_objects_complete_without_a_fingerprint() {
    let analysis = analyze(Some(&sqlite_tree()), Some(&references(&[])));
    assert_eq!(complete(analysis), None);
    assert!(!needs_schema(Some(&references(&[]))), "no schema is needed");
}

#[test]
fn fingerprint_is_order_independent_over_objects() {
    let sorted = analyze(
        Some(&sqlite_tree()),
        Some(&references(&[&["events"], &["orders"]])),
    );
    let shuffled = analyze(
        Some(&sqlite_tree()),
        Some(&references(&[&["orders"], &["events"]])),
    );
    assert_eq!(complete(sorted), complete(shuffled));
}

#[test]
fn fingerprint_changes_when_a_column_is_added() {
    let before = analyze(Some(&sqlite_tree()), Some(&references(&[&["events"]])));
    let mut changed = sqlite_tree();
    changed.databases[0].schemas[0].tables[0]
        .columns
        .push(Column {
            name: "extra".into(),
            data_type: "text".into(),
            nullable: true,
        });
    let after = analyze(Some(&changed), Some(&references(&[&["events"]])));
    assert_ne!(
        complete(before),
        complete(after),
        "an added column invalidates the review"
    );
}

#[test]
fn missing_table_is_unverifiable_not_a_constant() {
    // A2 decision 4: a missing object makes the review unverifiable — it can
    // never again contribute a constant digest that survives table changes.
    let analysis = analyze(
        Some(&sqlite_tree()),
        Some(&references(&[&["events"], &["ghost"]])),
    );
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"table "ghost" not found"#), "{reason}");
}

#[test]
fn missing_and_ambiguous_problems_are_named_together() {
    let tree = tree(vec![(
        "data",
        vec![("main", vec!["t"]), ("archive", vec!["t"])],
    )]);
    let analysis = analyze(Some(&tree), Some(&references(&[&["ghost"], &["t"]])));
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"table "ghost" not found"#), "{reason}");
    assert!(reason.contains(r#"ambiguous name "t""#), "{reason}");
}

#[test]
fn same_unqualified_name_in_two_schemas_is_unverifiable() {
    let tree = tree(vec![(
        "data",
        vec![("main", vec!["t"]), ("archive", vec!["t"])],
    )]);
    let analysis = analyze(Some(&tree), Some(&references(&[&["t"]])));
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"ambiguous name "t""#), "{reason}");
    // A qualified name still resolves exactly within its schema.
    let analysis = analyze(Some(&tree), Some(&references(&[&["archive", "t"]])));
    assert!(complete(analysis).is_some());
}

#[test]
fn case_sensitive_identifiers_resolve_exactly() {
    // Two tables differing only by case (a dialect that allows it): each
    // exact-case name resolves to its own table, and a name matching both
    // only case-insensitively is ambiguous.
    let tree = tree_of_tables(vec![(
        "data",
        vec![(
            "main",
            vec![table("Orders", &["alpha"]), table("orders", &["beta"])],
        )],
    )]);
    let mixed = unverifiable(analyze(Some(&tree), Some(&references(&[&["ORDERS"]]))));
    assert!(mixed.contains(r#"ambiguous name "ORDERS""#), "{mixed}");
    let upper = complete(analyze(Some(&tree), Some(&references(&[&["Orders"]]))));
    let lower = complete(analyze(Some(&tree), Some(&references(&[&["orders"]]))));
    assert_ne!(
        upper, lower,
        "each exact-case name resolved to its own table's fingerprint"
    );
}

#[test]
fn quoted_dotted_table_is_one_part_and_resolves() {
    // A2 decision 1: the parts arrive as the SQL wrote them and are never
    // re-split, so `"orders.v1"` is one object that resolves as a whole.
    let tree = tree_of_tables(vec![(
        "data",
        vec![("main", vec![table("orders.v1", &["id"])])],
    )]);
    let analysis = analyze(Some(&tree), Some(&references(&[&["orders.v1"]])));
    assert!(
        complete(analysis).is_some(),
        "the dotted name resolves whole"
    );
}

#[test]
fn dotted_name_is_never_split_into_schema_and_table() {
    // The F3 shape: a schema `orders` and a table `v1` exist, but the SQL
    // named the single table `orders.v1` — splitting it must not resolve.
    let tree = tree(vec![("data", vec![("orders", vec!["v1"])])]);
    let analysis = analyze(Some(&tree), Some(&references(&[&["orders.v1"]])));
    let reason = unverifiable(analysis);
    assert!(
        reason.contains(r#"table "orders.v1" not found"#),
        "{reason}"
    );
}

#[test]
fn partial_analysis_is_unverifiable() {
    let analysis = analyze(
        Some(&sqlite_tree()),
        Some(&partial_references(&[&["events"]])),
    );
    assert_eq!(unverifiable(analysis), "dependency analysis incomplete");
    assert!(
        !needs_schema(Some(&partial_references(&[&["events"]]))),
        "an incomplete analysis never fetches the schema"
    );
}

#[test]
fn absent_analysis_is_unverifiable() {
    let analysis = analyze(Some(&sqlite_tree()), None);
    assert_eq!(unverifiable(analysis), "dependency analysis incomplete");
    assert!(!needs_schema(None));
}
