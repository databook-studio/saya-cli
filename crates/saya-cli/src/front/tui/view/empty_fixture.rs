//! The facts each `02` §9 state is drawn from, and the readings a snapshot
//! takes of the rows.
//!
//! The connected database's questions are today's `starter_questions` for a
//! demo-shaped schema, so a test can compare the pane against the function
//! rather than against a list copied beside it.

use ratatui::text::Line;
use saya_types::{Column, Database as SchemaDatabase, Schema, SchemaTree, Table};

use super::empty::{Database, EmptyFacts};
use crate::front::tui::theme::{Charset, Scheme};
use crate::interactive::tui::ui::surface::starter_questions;
use crate::interactive::tui::wrap::cell_width;

/// `02` §9.3's demo database, as the header names it.
pub(super) fn demo() -> Database {
    Database {
        name: "demo".to_owned(),
        dialect: "sqlite".to_owned(),
        tables: 12,
    }
}

/// A table with the given columns and no keys.
fn table(name: &str, columns: &[(&str, &str)]) -> Table {
    Table {
        name: name.to_owned(),
        columns: columns
            .iter()
            .map(|(name, data_type)| Column {
                name: (*name).to_owned(),
                data_type: (*data_type).to_owned(),
                nullable: false,
            })
            .collect(),
        primary_key: Vec::new(),
        foreign_keys: Vec::new(),
    }
}

/// The schema the connected examples come from: three questions, one per
/// table, and the internal table the picker never asks about.
pub(super) fn demo_schema() -> SchemaTree {
    let mut customers = table(
        "customers",
        &[("id", "INTEGER"), ("name", "TEXT"), ("signup_date", "TEXT")],
    );
    customers.primary_key = vec!["id".to_owned()];
    let mut orders = table(
        "orders",
        &[
            ("id", "INTEGER"),
            ("order_date", "TEXT"),
            ("amount_cents", "INTEGER"),
        ],
    );
    orders.primary_key = vec!["id".to_owned()];
    let contacts = table(
        "customer_contacts",
        &[("customer_id", "INTEGER"), ("channel", "TEXT")],
    );
    let meta = table("saya_demo_meta", &[("key", "TEXT"), ("value", "TEXT")]);
    SchemaTree {
        databases: vec![SchemaDatabase {
            name: "demo".to_owned(),
            schemas: vec![Schema {
                name: "main".to_owned(),
                tables: vec![meta, orders, customers, contacts],
            }],
        }],
    }
}

/// The facts of a run that has nothing configured yet: `02` §9.1.
pub(super) fn first_run() -> EmptyFacts {
    EmptyFacts::default()
}

/// The facts of a ready run inside a project, with no database: `02` §9.2.
pub(super) fn no_database() -> EmptyFacts {
    EmptyFacts {
        provider: true,
        workspace: Some("~/shop".to_owned()),
        ..Default::default()
    }
}

/// The facts of a ready run with the demo database connected: `02` §9.3.
pub(super) fn connected() -> EmptyFacts {
    EmptyFacts {
        database: Some(demo()),
        starter_questions: starter_questions(&demo_schema()),
        ..no_database()
    }
}

/// The facts of a ready run outside any project, with a database: `02` §9.4.
pub(super) fn no_workspace() -> EmptyFacts {
    EmptyFacts {
        database: Some(demo()),
        ..EmptyFacts {
            provider: true,
            ..Default::default()
        }
    }
}

/// The same facts painted with no colour at all.
pub(super) fn colour_off(facts: &EmptyFacts) -> EmptyFacts {
    EmptyFacts {
        scheme: Scheme::NoColour,
        ..facts.clone()
    }
}

/// The same facts drawn with the ASCII glyph set.
pub(super) fn ascii(facts: &EmptyFacts) -> EmptyFacts {
    EmptyFacts {
        charset: Charset::Ascii,
        ..facts.clone()
    }
}

/// A connected database whose one question is longer than any pane: the wrap's
/// fixture.
pub(super) fn a_long_question() -> EmptyFacts {
    EmptyFacts {
        starter_questions: vec![
            "How does the total amount of every completed order change by the order date in \
             the orders table over the last twelve months?"
                .to_owned(),
        ],
        ..connected()
    }
}

/// The rows as a terminal paints them, one string per row.
pub(super) fn rendered(rows: &[Line<'_>]) -> Vec<String> {
    rows.iter().map(words).collect()
}

/// One row as a terminal paints it.
pub(super) fn words(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The cells one row occupies.
pub(super) fn cells(line: &Line<'_>) -> usize {
    cell_width(&words(line))
}
