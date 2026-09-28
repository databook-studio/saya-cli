//! Unit tests for the schema review (`fingerprint::analyze`, A2): the
//! definition's SQL yields the referenced parts, each resolves against the
//! live schema tree as exactly one table under the dialect's identifier
//! rules (re-audit R1: quoting + case folding), missing or ambiguous
//! resolution makes the whole review unverifiable, and a complete review
//! folds the resolved per-table digests into one combined value.

use super::{Analysis, analyze, needs_schema};
use saya_connectors::SqlReferences;
use saya_types::{Column, Database, Schema, SchemaTree, SqlDialect, Table};

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

/// References as `sql_references` reports them: parts as written, complete,
/// every part bare in the SQL.
fn references(objects: &[&[&str]]) -> SqlReferences {
    references_with_quoting(objects, false)
}

/// References whose parts were all quoted in the SQL.
fn quoted_references(objects: &[&[&str]]) -> SqlReferences {
    references_with_quoting(objects, true)
}

fn references_with_quoting(objects: &[&[&str]], quoted: bool) -> SqlReferences {
    SqlReferences {
        objects: objects
            .iter()
            .map(|parts| parts.iter().map(|part| (*part).to_string()).collect())
            .collect(),
        object_quoting: objects
            .iter()
            .map(|parts| vec![quoted; parts.len()])
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
    let analysis = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"]])),
    );
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
        SqlDialect::Postgres,
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
        SqlDialect::Postgres,
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
        SqlDialect::Postgres,
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

/// A Postgres-shaped tree holding both spellings of the same table, as when
/// one was created unquoted and the other quoted.
fn orders_tree() -> SchemaTree {
    tree_of_tables(vec![(
        "analytics",
        vec![(
            "public",
            vec![table("orders", &["a"]), table("Orders", &["b"])],
        )],
    )])
}

/// Renames the first column of one table, so the table's fingerprint changes.
fn stale_column(tree: &mut SchemaTree, table_index: usize) {
    tree.databases[0].schemas[0].tables[table_index].columns[0].name = "staled".into();
}

#[test]
fn postgres_unquoted_orders_fingerprints_the_lowercase_table() {
    // Re-audit R1: unquoted `Orders` folds to `orders`, so the review binds
    // the real `orders` — changing `orders` stales the review, changing the
    // quoted `Orders` table does not.
    let plain = complete(analyze(
        Some(&orders_tree()),
        SqlDialect::Postgres,
        Some(&references(&[&["Orders"]])),
    ))
    .expect("unquoted `Orders` folds to the existing `orders`");
    let mut staled = orders_tree();
    stale_column(&mut staled, 0);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_ne!(plain, after, "changing `orders` must stale the review");
    let mut staled = orders_tree();
    stale_column(&mut staled, 1);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_eq!(plain, after, "changing `Orders` must not stale the review");
}

#[test]
fn postgres_quoted_orders_fingerprints_the_exact_case_table() {
    // The reverse: quoted `"Orders"` compares exactly, so changing the
    // quoted `Orders` table stales the review and changing `orders` does not.
    let quoted = complete(analyze(
        Some(&orders_tree()),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["Orders"]])),
    ))
    .expect("quoted `\"Orders\"` matches the exact-case table");
    let mut staled = orders_tree();
    stale_column(&mut staled, 1);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_ne!(quoted, after, "changing `Orders` must stale the review");
    let mut staled = orders_tree();
    stale_column(&mut staled, 0);
    let after = complete(analyze(
        Some(&staled),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["Orders"]])),
    ))
    .expect("still resolves after the change");
    assert_eq!(quoted, after, "changing `orders` must not stale the review");
}

#[test]
fn postgres_quoted_parts_are_never_folded() {
    // `"PUBLIC".orders` is not `public.orders`: a quoted part compares
    // exactly, so it must not resolve — while the bare-spelled name does.
    let refs = SqlReferences {
        objects: vec![vec!["PUBLIC".to_string(), "orders".to_string()]],
        object_quoting: vec![vec![true, false]],
        columns: Vec::new(),
        partial: false,
    };
    let reason = unverifiable(analyze(
        Some(&postgres_tree()),
        SqlDialect::Postgres,
        Some(&refs),
    ));
    assert!(reason.contains("not found"), "{reason}");
    let analysis = analyze(
        Some(&postgres_tree()),
        SqlDialect::Postgres,
        Some(&quoted_references(&[&["public", "events"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "the exact-case quote resolves"
    );
}

#[test]
fn snowflake_folds_unquoted_names_to_uppercase() {
    // Snowflake stores unquoted identifiers upper-cased: the unquoted SQL
    // `orders` IS the tree's `ORDERS`; a quoted `orders` is not.
    let tree = tree(vec![("analytics", vec![("public", vec!["ORDERS"])])]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Snowflake,
        Some(&references(&[&["orders"]])),
    );
    assert!(
        complete(analysis).is_some(),
        "unquoted `orders` folds to `ORDERS` and resolves"
    );
    let reason = unverifiable(analyze(
        Some(&tree),
        SqlDialect::Snowflake,
        Some(&quoted_references(&[&["orders"]])),
    ));
    assert!(reason.contains(r#"table "orders" not found"#), "{reason}");
}

#[test]
fn case_insensitive_dialects_fail_closed_on_case_collisions() {
    // SQLite/MySQL/DuckDB case behaviour is engine- or setting-dependent:
    // when more than one tree name matches case-insensitively the review
    // refuses — the spelling never picks among candidates (decision 3).
    let tree = tree_of_tables(vec![(
        "data",
        vec![(
            "main",
            vec![table("orders", &["a"]), table("Orders", &["b"])],
        )],
    )]);
    for dialect in [SqlDialect::Sqlite, SqlDialect::Mysql, SqlDialect::DuckDb] {
        for asked in ["orders", "Orders", "ORDERS"] {
            let reason = unverifiable(analyze(
                Some(&tree),
                dialect,
                Some(&references(&[&[asked]])),
            ));
            assert!(
                reason.contains(r#"ambiguous name ""#) && reason.contains(asked),
                "{dialect:?} {asked}: {reason}"
            );
        }
    }
}

#[test]
fn sqlite_single_table_resolves_in_any_case() {
    // One candidate: SQLite's case-insensitive matching resolves it however
    // the SQL spelled it.
    let tree = sqlite_tree();
    for asked in ["events", "EVENTS", "Events"] {
        let analysis = analyze(
            Some(&tree),
            SqlDialect::Sqlite,
            Some(&references(&[&[asked]])),
        );
        assert!(complete(analysis).is_some(), "{asked} must resolve");
    }
}

#[test]
fn clickhouse_matches_case_sensitively_regardless_of_quoting() {
    // ClickHouse never folds: only the exact-case tree name resolves, quoted
    // or not, and each exact-case name binds its own table's fingerprint.
    let tree = tree_of_tables(vec![(
        "analytics",
        vec![(
            "analytics",
            vec![table("orders", &["a"]), table("Orders", &["b"])],
        )],
    )]);
    let lower = complete(analyze(
        Some(&tree),
        SqlDialect::ClickHouse,
        Some(&references(&[&["analytics", "orders"]])),
    ))
    .expect("the exact-case bare name resolves");
    let upper = complete(analyze(
        Some(&tree),
        SqlDialect::ClickHouse,
        Some(&quoted_references(&[&["analytics", "Orders"]])),
    ))
    .expect("the exact-case quoted name resolves");
    assert_ne!(
        lower, upper,
        "each exact-case name resolved to its own table"
    );
    let reason = unverifiable(analyze(
        Some(&tree),
        SqlDialect::ClickHouse,
        Some(&references(&[&["analytics", "ORDERS"]])),
    ));
    assert!(
        reason.contains(r#"table "analytics.ORDERS" not found"#),
        "{reason}"
    );
}

#[test]
fn zero_objects_complete_without_a_fingerprint() {
    let analysis = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[])),
    );
    assert_eq!(complete(analysis), None);
    assert!(!needs_schema(Some(&references(&[]))), "no schema is needed");
}

#[test]
fn fingerprint_is_order_independent_over_objects() {
    let sorted = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"], &["orders"]])),
    );
    let shuffled = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["orders"], &["events"]])),
    );
    assert_eq!(complete(sorted), complete(shuffled));
}

#[test]
fn fingerprint_changes_when_a_column_is_added() {
    let before = analyze(
        Some(&sqlite_tree()),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"]])),
    );
    let mut changed = sqlite_tree();
    changed.databases[0].schemas[0].tables[0]
        .columns
        .push(Column {
            name: "extra".into(),
            data_type: "text".into(),
            nullable: true,
        });
    let after = analyze(
        Some(&changed),
        SqlDialect::Sqlite,
        Some(&references(&[&["events"]])),
    );
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
        SqlDialect::Sqlite,
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
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&references(&[&["ghost"], &["t"]])),
    );
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
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&references(&[&["t"]])),
    );
    let reason = unverifiable(analysis);
    assert!(reason.contains(r#"ambiguous name "t""#), "{reason}");
    // A qualified name still resolves exactly within its schema.
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&references(&[&["archive", "t"]])),
    );
    assert!(complete(analysis).is_some());
}

#[test]
fn quoted_dotted_table_is_one_part_and_resolves() {
    // A2 decision 1: the parts arrive as the SQL wrote them and are never
    // re-split, so `"orders.v1"` is one object that resolves as a whole.
    let tree = tree_of_tables(vec![(
        "data",
        vec![("main", vec![table("orders.v1", &["id"])])],
    )]);
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&quoted_references(&[&["orders.v1"]])),
    );
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
    let analysis = analyze(
        Some(&tree),
        SqlDialect::Sqlite,
        Some(&quoted_references(&[&["orders.v1"]])),
    );
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
        SqlDialect::Sqlite,
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
    let analysis = analyze(Some(&sqlite_tree()), SqlDialect::Sqlite, None);
    assert_eq!(unverifiable(analysis), "dependency analysis incomplete");
    assert!(!needs_schema(None));
}
