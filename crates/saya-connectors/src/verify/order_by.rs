//! Detect a top-level `ORDER BY` clause by parsing, not string matching.
//!
//! Used by the candidate-decision layer to decide whether row order is part
//! of an answer. Only the outermost query's `ORDER BY` counts; one buried in a
//! subquery does not, and the literal text `'order by'` inside a string
//! constant is invisible because the SQL is parsed to an AST before inspection.

use saya_types::SqlDialect;
use sqlparser::ast::Statement;
use sqlparser::parser::Parser;

use crate::safety::parser_dialect;

/// True when `sql` is a single query that carries an outermost `ORDER BY`.
///
/// A statement that is not a single `SELECT` query, or that fails to parse,
/// reports `false`: a parse failure cannot establish an `ORDER BY`, so it does
/// not contribute one. A set operation (`UNION`/`INTERSECT`/`EXCEPT`) with an
/// outer `ORDER BY` is top-level, since the ordering applies to the whole set.
pub fn has_top_level_order_by(sql: &str, dialect: SqlDialect) -> bool {
    let Ok(mut statements) = Parser::parse_sql(parser_dialect(dialect), sql) else {
        return false;
    };
    if statements.len() != 1 {
        return false;
    }
    matches!(statements.remove(0), Statement::Query(q) if q.order_by.is_some())
}
