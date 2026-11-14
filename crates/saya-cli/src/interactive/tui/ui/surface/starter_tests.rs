//! `starter_questions`' contract: deterministic, metadata-only prompts —
//! tables in name order, system tables skipped, control characters stripped,
//! long questions capped. The fixtures mirror the shapes real connectors
//! produce: SQLite populates primary and foreign keys; the other connectors
//! may leave both empty, and the questions degrade to the simpler shapes.

use super::*;
use saya_types::{Column, Database, ForeignKey, Schema, SchemaTree, Table};

/// A table with the given columns and no keys.
fn table(name: &str, columns: &[(&str, &str)]) -> Table {
    Table {
        name: name.to_string(),
        columns: columns
            .iter()
            .map(|(name, data_type)| Column {
                name: (*name).to_string(),
                data_type: (*data_type).to_string(),
                nullable: false,
            })
            .collect(),
        primary_key: Vec::new(),
        foreign_keys: Vec::new(),
    }
}

/// A single-database tree holding `tables` verbatim — deliberately unsorted,
/// so the questions must come out sorted.
fn tree(tables: Vec<Table>) -> SchemaTree {
    SchemaTree {
        databases: vec![Database {
            name: "demo".to_string(),
            schemas: vec![Schema {
                name: "main".to_string(),
                tables,
            }],
        }],
    }
}

/// The demo fixture's foreign-key shape: `from` references `customers(id)`.
fn references_customers(from: &str) -> ForeignKey {
    ForeignKey {
        columns: vec![from.to_string()],
        referenced_schema: None,
        referenced_table: "customers".to_string(),
        referenced_columns: vec!["id".to_string()],
    }
}

/// The demo fixture's shape (customers / orders / customer_contacts, plus the
/// `saya_`-prefixed meta table): three questions, one per table in name order,
/// the internal table skipped — and the same tree yields the same questions
/// again.
#[test]
fn demo_like_schema_yields_three_deterministic_questions() {
    let mut customers = table(
        "customers",
        &[
            ("id", "INTEGER"),
            ("name", "TEXT"),
            ("email", "TEXT"),
            ("region", "TEXT"),
            ("signup_date", "TEXT"),
            ("status", "TEXT"),
        ],
    );
    customers.primary_key = vec!["id".to_string()];
    let mut orders = table(
        "orders",
        &[
            ("id", "INTEGER"),
            ("customer_id", "INTEGER"),
            ("order_date", "TEXT"),
            ("amount_cents", "INTEGER"),
            ("status", "TEXT"),
        ],
    );
    orders.primary_key = vec!["id".to_string()];
    orders.foreign_keys = vec![references_customers("customer_id")];
    let mut contacts = table(
        "customer_contacts",
        &[
            ("customer_id", "INTEGER"),
            ("channel", "TEXT"),
            ("value", "TEXT"),
        ],
    );
    contacts.foreign_keys = vec![references_customers("customer_id")];
    let mut meta = table("saya_demo_meta", &[("key", "TEXT"), ("value", "TEXT")]);
    meta.primary_key = vec!["key".to_string()];
    let schema = tree(vec![meta, orders, customers, contacts]);
    let questions = starter_questions(&schema);
    assert_eq!(
        questions,
        vec![
            "What are the most common channel values in customer_contacts?",
            "What are the most common name values in customers?",
            "How does amount_cents change by order_date in orders?",
        ]
    );
    assert_eq!(
        starter_questions(&schema),
        questions,
        "the same tree yields the same questions"
    );
}

/// A numeric column that is neither key nor reference plus a date column —
/// by declared type or by name — yields the trend question.
#[test]
fn a_measure_and_a_date_column_yield_the_trend_question() {
    let mut invoices = table(
        "invoices",
        &[
            ("id", "BIGINT"),
            ("total_cents", "BIGINT"),
            ("issued_at", "TIMESTAMP"),
        ],
    );
    invoices.primary_key = vec!["id".to_string()];
    assert_eq!(
        starter_questions(&tree(vec![invoices])),
        vec!["How does total_cents change by issued_at in invoices?"],
    );
}

/// A table whose only numeric column is its identifier has no measure to
/// trend; with no text column either, the question is the row count.
#[test]
fn a_table_without_a_measure_or_text_gets_the_row_count_question() {
    let mut events = table("events", &[("id", "INTEGER")]);
    events.primary_key = vec!["id".to_string()];
    assert_eq!(
        starter_questions(&tree(vec![events])),
        vec!["How many rows are in events?"],
    );
}

/// Nothing in an empty tree — or a database with no tables — can be asked
/// about: fewer than three is correct when the schema supports fewer.
#[test]
fn an_empty_schema_yields_no_questions() {
    assert!(starter_questions(&SchemaTree::default()).is_empty());
    assert!(starter_questions(&tree(Vec::new())).is_empty());
}

/// Internal tables never become starter questions, whatever case the
/// connector surfaces them in.
#[test]
fn system_tables_are_skipped() {
    let schema = tree(vec![
        table("saya_demo_meta", &[("key", "TEXT"), ("value", "TEXT")]),
        table("pg_stat_activity", &[("pid", "INTEGER")]),
        table("sqlite_sequence", &[("seq", "INTEGER")]),
        table("information_schema_columns", &[("name", "TEXT")]),
        table("SQLite_stat1", &[("idx", "TEXT")]),
        table("customers", &[("name", "TEXT")]),
    ]);
    assert_eq!(
        starter_questions(&schema),
        vec!["What are the most common name values in customers?"],
    );
}

/// Database metadata is untrusted: control characters never reach the
/// starter line, while the visible characters stay exactly as written.
#[test]
fn control_characters_are_stripped_from_names() {
    let schema = tree(vec![table("orders\n", &[("regi\u{7}on", "TEXT")])]);
    assert_eq!(
        starter_questions(&schema),
        vec!["What are the most common region values in orders?"],
    );
}

/// A name past the question cap truncates: the question is exactly 120
/// chars, never longer.
#[test]
fn long_names_are_capped_at_120_chars() {
    let schema = tree(vec![table(&"t".repeat(200), &[("id", "INTEGER")])]);
    let questions = starter_questions(&schema);
    assert_eq!(questions.len(), 1);
    assert_eq!(questions[0].chars().count(), 120);
}

/// Two databases holding a same-named table produce the same question; the
/// duplicate is dropped rather than shown twice.
#[test]
fn identical_questions_from_same_named_tables_are_not_repeated() {
    let schema = SchemaTree {
        databases: vec![
            Database {
                name: "one".to_string(),
                schemas: vec![Schema {
                    name: "public".to_string(),
                    tables: vec![table("orders", &[("id", "INTEGER")])],
                }],
            },
            Database {
                name: "two".to_string(),
                schemas: vec![Schema {
                    name: "public".to_string(),
                    tables: vec![table("orders", &[("id", "INTEGER")])],
                }],
            },
        ],
    };
    assert_eq!(
        starter_questions(&schema),
        vec!["How many rows are in orders?"],
    );
}
