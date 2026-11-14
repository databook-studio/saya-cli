//! Tests for `references`, grouped by responsibility: which names are
//! extracted from a statement (`extraction`), where the extractor's bounds
//! bite (`limits`), CTE alias visibility (`cte_scoping`), and
//! quoted-identifier recording (`quoting`).

use super::*;

/// Postgres accepts the most syntax (catalog-qualified names, CTEs), so it
/// is the default dialect for these tests unless a case is dialect-specific.
const D: SqlDialect = SqlDialect::Postgres;

fn objects(sql: &str) -> Vec<Vec<String>> {
    sql_references(sql, D).expect("should parse").objects
}
fn columns(sql: &str) -> Vec<String> {
    sql_references(sql, D).expect("should parse").columns
}
fn partial(sql: &str) -> bool {
    sql_references(sql, D).expect("should parse").partial
}
fn quoting(sql: &str) -> Vec<Vec<bool>> {
    sql_references(sql, D).expect("should parse").object_quoting
}

mod cte_scoping;
mod extraction;
mod limits;
mod quoting;
