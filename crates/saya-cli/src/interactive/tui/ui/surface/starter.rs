//! Schema-derived starter questions for the empty state's examples section:
//! deterministic prompts built from table and column metadata alone — never
//! row data, never a model call, never I/O.
//!
//! Wiring: `application::picker::reload_at_refs` builds these from the active
//! profile's schema cached in the state store (at startup, on resume, and
//! when a newly loaded schema lands), and the empty state paints them.
//!
//! How the questions are picked, so the output can be reasoned about
//! without running it: tables in name order across the tree, one question
//! per table, internal tables skipped, exact duplicates dropped, stopping
//! at three. Each table gets the richest shape its metadata supports — a
//! trend when it has both a numeric measure and a date column, else the
//! most common values of a text column, else the row count. A measure is a
//! numeric column that is neither a primary-key part nor a foreign-key
//! column: identifiers do not trend. Date-ness reads the declared type or
//! the column's name (`date`/`time`, or a `_at` suffix), because fixture
//! dialects store dates as text. Connectors that leave key metadata empty
//! may nominate an identifier column as a measure; the question stays
//! honest — it is built from metadata — just less curated.

use saya_types::{SchemaTree, Table};

/// The most questions shown; the empty state's examples section holds three.
const MAX_QUESTIONS: usize = 3;

/// A question longer than this wraps the centred splash; the cap counts
/// chars, so it holds whatever the names contain.
const MAX_QUESTION_CHARS: usize = 120;

/// Internal tables are never suggested: their names start with one of these
/// prefixes, matched case-insensitively — connectors surface the internals
/// of different dialects in different cases.
const SYSTEM_TABLE_PREFIXES: [&str; 4] = ["sqlite_", "pg_", "information_schema", "saya_"];

const TEXT_TYPES: [&str; 5] = ["char", "text", "clob", "string", "uuid"];
const NUMERIC_TYPES: [&str; 8] = [
    "int", "float", "double", "decimal", "numeric", "number", "real", "money",
];
const DATE_TYPES: [&str; 2] = ["date", "time"];

/// Up to three deterministic starter questions for `tree`.
pub(crate) fn starter_questions(tree: &SchemaTree) -> Vec<String> {
    let mut questions: Vec<String> = Vec::new();
    for database in &tree.databases {
        for schema in &database.schemas {
            let mut tables: Vec<&Table> = schema.tables.iter().collect();
            tables.sort_by(|a, b| a.name.cmp(&b.name));
            for table in tables {
                if is_system_table(&table.name) {
                    continue;
                }
                let question = question_for(table);
                if !questions.contains(&question) {
                    questions.push(question);
                    if questions.len() == MAX_QUESTIONS {
                        return questions;
                    }
                }
            }
        }
    }
    questions
}

fn is_system_table(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SYSTEM_TABLE_PREFIXES
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

fn question_for(table: &Table) -> String {
    let measure = table.columns.iter().find(|column| {
        is_typed(&column.data_type, &NUMERIC_TYPES) && !is_key_column(table, &column.name)
    });
    let date = table
        .columns
        .iter()
        .find(|column| is_typed(&column.data_type, &DATE_TYPES) || is_date_named(&column.name));
    let text = table
        .columns
        .iter()
        .find(|column| is_typed(&column.data_type, &TEXT_TYPES));
    let raw = if let (Some(measure), Some(date)) = (measure, date) {
        format!(
            "How does {} change by {} in {}?",
            measure.name, date.name, table.name
        )
    } else if let Some(text) = text {
        format!(
            "What are the most common {} values in {}?",
            text.name, table.name
        )
    } else {
        format!("How many rows are in {}?", table.name)
    };
    clean(&raw)
}

fn is_key_column(table: &Table, column: &str) -> bool {
    table.primary_key.iter().any(|key| key == column)
        || table
            .foreign_keys
            .iter()
            .any(|key| key.columns.iter().any(|part| part == column))
}

fn is_date_named(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("date") || lower.contains("time") || lower.ends_with("_at")
}

fn is_typed(data_type: &str, markers: &[&str]) -> bool {
    let lower = data_type.to_ascii_lowercase();
    markers.iter().any(|marker| lower.contains(marker))
}

/// Strips control characters — database metadata is untrusted and must never
/// reach the terminal raw — and caps the question at 120 chars.
fn clean(question: &str) -> String {
    question
        .chars()
        .filter(|ch| !ch.is_control())
        .take(MAX_QUESTION_CHARS)
        .collect()
}

#[cfg(test)]
#[path = "starter_tests.rs"]
mod tests;
