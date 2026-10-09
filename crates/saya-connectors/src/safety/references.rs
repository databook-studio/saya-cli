//! Deterministic object and column extraction from a parsed SQL statement.
//!
//! See `.claude/specs/spec-3b1-sql-object-extraction.md`. The output is names
//! only — no SQL is persisted or returned — which is the property that lets a
//! caller store the result as an observation.

use saya_types::SqlDialect;
use sqlparser::ast::{Statement, Visit};
use sqlparser::parser::Parser;

use super::read_only::parser_dialect;

/// At most this many objects/columns are recorded; more sets [`SqlReferences::partial`].
const MAX_OBJECTS: usize = 64;
const MAX_COLUMNS: usize = 256;

/// Objects and columns referenced by a single SQL statement, as written.
///
/// `partial` is `true` when the statement parsed but used a construct this
/// extractor does not model, so `objects`/`columns` may be incomplete. A caller
/// must treat a partial result as "at least these, possibly more" and never as
/// an exhaustive list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SqlReferences {
    /// Table-ish names as written, each split into its parts (1, 2 or 3).
    pub objects: Vec<Vec<String>>,
    /// For each part of each object, `true` when the SQL quoted it (`"x"`,
    /// `` `x` ``, `[x]`) — the engine then matches that identifier exactly
    /// rather than applying its unquoted folding rules. Same shape as
    /// `objects`.
    pub object_quoting: Vec<Vec<bool>>,
    /// Column identifiers as written. Unqualified names appear bare.
    pub columns: Vec<String>,
    /// `true` when parsing succeeded but the statement used a construct this
    /// extractor does not model, so the lists may be incomplete.
    pub partial: bool,
}

/// Extract the object and column names a single SQL statement references.
///
/// Returns `None` when the SQL does not parse; a caller must not guess from
/// unparseable input. Parsed but unmodelled constructs set `partial = true`
/// rather than emitting a name the extractor is unsure about.
pub fn sql_references(sql: &str, dialect: SqlDialect) -> Option<SqlReferences> {
    let mut statements = Parser::parse_sql(parser_dialect(dialect), sql).ok()?;
    // A caller attributes one observation to one statement; multiple statements
    // in one string is ambiguous, so refuse rather than merge.
    if statements.len() != 1 {
        return None;
    }
    let query = match statements.remove(0) {
        Statement::Query(q) => q,
        // `EXPLAIN <query>` extracts from the underlying query; `EXPLAIN` of
        // anything else is parsed but not modelled.
        Statement::Explain { statement, .. } => match *statement {
            Statement::Query(q) => q,
            _ => {
                return Some(SqlReferences {
                    partial: true,
                    ..SqlReferences::default()
                });
            }
        },
        // DML/DDL/SHOW parse but are not modelled: empty lists, partial.
        _ => {
            return Some(SqlReferences {
                partial: true,
                ..SqlReferences::default()
            });
        }
    };
    let mut ex = Extractor::default();
    // The visitor never breaks, so a `Break` here would mean we stopped before
    // seeing the whole tree — the lists may be incomplete.
    let stopped_early = query.visit(&mut ex).is_break();
    Some(SqlReferences {
        objects: ex.objects,
        object_quoting: ex.object_quoting,
        columns: ex.columns,
        partial: ex.partial || stopped_early,
    })
}

mod walk;
use walk::Extractor;

#[cfg(test)]
mod tests;
