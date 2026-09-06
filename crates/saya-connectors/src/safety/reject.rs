//! Why the read-only safety layer rejected a statement, and how to say so.
//!
//! Rejections surface verbatim to the human *and* to the agent, which uses
//! the reason to self-correct on its next turn. Every message names what was
//! wrong and what is allowed instead — never just "rejected".

use saya_types::ConnectionError;
use sqlparser::ast::Statement;

pub(super) enum Rejection {
    Parse,
    MultipleStatements,
    WriteStatement(&'static str),
    Denied(String),
    RowCap,
    LockingClause,
    FormatClause,
}

pub(super) fn rejected(reason: Rejection) -> ConnectionError {
    let detail = match reason {
        Rejection::Parse => {
            "not parseable as one read-only statement; only SELECT, SHOW, and EXPLAIN are allowed"
                .to_string()
        }
        Rejection::MultipleStatements => {
            "only one statement is allowed per request — run them one at a time".to_string()
        }
        Rejection::WriteStatement(kind) => {
            format!("{kind} modifies data or schema; only reads are allowed")
        }
        Rejection::Denied(name) => {
            format!("{name} can mutate state or reach outside this database and is not allowed")
        }
        Rejection::RowCap => "the row limit must be at least 1".to_string(),
        Rejection::LockingClause => {
            "FOR UPDATE/FOR SHARE takes row locks; a plain SELECT is required".to_string()
        }
        Rejection::FormatClause => {
            "the FORMAT clause is reserved for the connector's own wire format; remove it from the query".to_string()
        }
    };
    ConnectionError::query_failed(format!(
        "query rejected by read-only safety policy: {detail}"
    ))
}

/// A short label for a statement that failed the allow-list.
pub(super) fn kind(statement: &Statement) -> &'static str {
    match statement {
        Statement::Insert(_) => "INSERT",
        Statement::Update { .. } => "UPDATE",
        Statement::Delete { .. } => "DELETE",
        Statement::CreateTable { .. } | Statement::CreateView { .. } => "CREATE",
        Statement::Drop { .. } => "DROP",
        Statement::AlterTable { .. } => "ALTER TABLE",
        Statement::Truncate { .. } => "TRUNCATE",
        Statement::Copy { .. } => "COPY",
        Statement::Grant { .. } | Statement::Revoke { .. } => "GRANT/REVOKE",
        _ => "this statement",
    }
}
