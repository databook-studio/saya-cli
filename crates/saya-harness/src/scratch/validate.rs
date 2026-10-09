//! The scratch validator: its own `sqlparser` pass with its own policy
//! (ADR 0003, amended 2026-09-11). It shares nothing with the connector's
//! read-only gate — no call, no extension, no parameter — because scratch is
//! the *writable* surface; any shared plumbing would be the thin end of a
//! conditional gate. Per the amendment: scratch opens with external access
//! off, and the validator rejects every file-reading function outright, by
//! name, as a category — no path-containment check, no in-run-dir carve-out;
//! corpus data reaches scratch through the workspace tools, not DuckDB. What
//! survives is the capability the ADR was written to get: DDL, DML and joins
//! on the scratch file itself, one statement per call, rows to the model
//! capped at the 50-row discipline.

mod allowlist;
mod file_reads;
mod row_cap;
mod types;

use sqlparser::ast::Visit;
use sqlparser::{dialect::DuckDbDialect, parser::Parser};

pub use self::types::{MAX_SQL_BYTES, SCRATCH_ROW_CAP, ScratchRejection, Validated};

use self::allowlist::allowed;
use self::file_reads::Guard;
use self::row_cap::{cap, top_query};

/// Validates one statement against scratch's own policy: the statement
/// allowlist (the file/configuration/extension family refused by kind), the
/// file-reader walk, then the single-statement and row-cap discipline.
pub fn validate(sql: &str) -> Result<Validated, ScratchRejection> {
    if sql.len() > MAX_SQL_BYTES {
        return Err(ScratchRejection::TooLarge {
            bytes: sql.len(),
            max: MAX_SQL_BYTES,
        });
    }
    let mut statements =
        Parser::parse_sql(&DuckDbDialect, sql).map_err(|_| ScratchRejection::Unparsable)?;
    if statements.len() != 1 {
        return Err(ScratchRejection::MultipleStatements {
            count: statements.len(),
        });
    }
    // The allowlist runs before the walk so a refused statement kind is reported
    // as its own kind — `INSTALL httpfs`, not "file function".
    let returns_rows = allowed(&statements[0])?;
    let mut guard = Guard { denied: None };
    if statements.visit(&mut guard).is_break() {
        return Err(guard
            .denied
            .unwrap_or_else(|| ScratchRejection::Statement("this statement".to_string())));
    }
    if let Some(query) = top_query(&mut statements[0]) {
        cap(query, SCRATCH_ROW_CAP);
    }
    Ok(Validated {
        sql: statements.remove(0).to_string(),
        returns_rows,
    })
}
