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

include!("fingerprint_tests/qualification.rs");

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

include!("fingerprint_tests/dialect_folding.rs");

include!("fingerprint_tests/fingerprint_stability.rs");

include!("fingerprint_tests/missing_and_ambiguous.rs");

include!("fingerprint_tests/quoted_identifiers.rs");

include!("fingerprint_tests/analysis_gaps.rs");
