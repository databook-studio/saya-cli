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

use std::ops::ControlFlow;

use sqlparser::{
    ast::{Expr, ObjectName, Query, Statement, TableFactor, Visit, Visitor},
    dialect::DuckDbDialect,
    parser::Parser,
};

/// Rows a scratch result may hand back to the model — the same small-sample
/// discipline as the interactive SQL tools: a larger result is truncated
/// (`truncated: true`) rather than shipped whole.
pub const SCRATCH_ROW_CAP: usize = 50;

/// Maximum SQL payload parsed by the scratch validator. The tool input is
/// untrusted; this pre-parse ceiling keeps the parser from being an allocation
/// bypass for callers that do not use the CLI input reader.
pub const MAX_SQL_BYTES: usize = 512 * 1024;

/// A statement the validator accepted, ready to execute.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Validated {
    /// The statement as it will run; reads carry an injected `LIMIT` when the statement did not bound itself.
    pub sql: String,
    /// True when the statement hands rows back (SELECT, EXPLAIN, SHOW,
    /// `INSERT … RETURNING`); false when it returns no rows.
    pub returns_rows: bool,
}

/// Why scratch refused a statement. Typed — the tool surfaces the variant's
/// own text so the model can self-correct — and deliberately its own type:
/// the connector's rejection vocabulary belongs to the user-database gate,
/// and scratch's does not parameterise it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ScratchRejection {
    #[error("scratch SQL exceeds the {max}-byte limit (received {bytes} bytes)")]
    TooLarge { bytes: usize, max: usize },
    #[error("scratch accepts exactly one statement; {count} were given — run them one at a time")]
    MultipleStatements { count: usize },
    #[error("scratch does not parse this as a single DuckDB statement")]
    Unparsable,
    #[error(
        "scratch refuses the file-reading function \"{0}\" — no file reader of any kind is allowed inside scratch; stage the data through the workspace tools instead"
    )]
    FileFunction(String),
    #[error(
        "scratch refuses \"{0}\" as a table name — a quoted file path is not a relation, and no file reads are allowed inside scratch"
    )]
    FilePathRelation(String),
    #[error(
        "scratch refuses {0} — the scratch engine holds no other files, no extensions, no attachments, and no configuration"
    )]
    Statement(String),
}

/// Every DuckDB reader of data outside the scratch file — local files, remote
/// URLs, other engines' files — starts with one of these prefixes: `read_csv`,
/// `read_parquet`, `read_json`, `http_get`, … Matched per identifier part
/// (lowercased, unquoted) so qualification or requoting cannot hide the name.
const FILE_READ_PREFIXES: &[&str] = &["read_", "http_"];

/// The readers that carry no prefix: table functions over files, other
/// engines, or the filesystem.
const FILE_READ_FUNCTIONS: &[&str] = &[
    "glob",
    "metadata",
    "parquet_scan",
    "parquet_metadata",
    "parquet_schema",
    "parquet_file_metadata",
    "parquet_kv_metadata",
    "sqlite_scan",
    "postgres_scan",
    "mysql_scan",
    "arrow",
    "arrow_scan",
    "iceberg_scan",
    "delta_scan",
];

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

/// The allowlist of statement shapes scratch accepts — reads, the write
/// capability (DDL/DML on the scratch file itself), and introspection;
/// everything else is refused by kind (`ATTACH`, `COPY … FROM`/`TO`,
/// `INSTALL`, `LOAD`, `PRAGMA`, `SET`, `CALL`, the file/configuration family).
/// Returns whether the accepted shape hands rows back.
fn allowed(statement: &Statement) -> Result<bool, ScratchRejection> {
    match statement {
        Statement::Query(_) => Ok(true),
        Statement::Explain { statement, .. } => match statement.as_ref() {
            Statement::Query(_) => Ok(true),
            other => Err(ScratchRejection::Statement(kind(other))),
        },
        Statement::ShowVariable { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowCollation { .. } => Ok(true),
        Statement::Insert(insert) => Ok(insert.returning.is_some()),
        Statement::CreateTable { .. }
        | Statement::CreateView { .. }
        | Statement::CreateSchema { .. }
        | Statement::CreateIndex { .. }
        | Statement::Update { .. }
        | Statement::Delete { .. }
        | Statement::Drop { .. }
        | Statement::AlterTable { .. }
        | Statement::Truncate { .. }
        | Statement::Merge { .. }
        | Statement::Analyze { .. } => Ok(false),
        other => Err(ScratchRejection::Statement(kind(other))),
    }
}

/// A specific label for the refused statement family — `INSTALL httpfs`,
/// `ATTACH`, `COPY … TO` — so the refusal names what was wrong.
fn kind(statement: &Statement) -> String {
    match statement {
        Statement::Install { extension_name } => format!("INSTALL {}", extension_name.value),
        Statement::Load { extension_name } => format!("LOAD {}", extension_name.value),
        Statement::AttachDatabase { .. } | Statement::AttachDuckDBDatabase { .. } => {
            "ATTACH".to_string()
        }
        Statement::DetachDuckDBDatabase { .. } => "DETACH".to_string(),
        Statement::Copy { to: true, .. } => "COPY … TO".to_string(),
        Statement::Copy { .. } => "COPY … FROM".to_string(),
        Statement::SetVariable { .. }
        | Statement::SetTimeZone { .. }
        | Statement::SetRole { .. }
        | Statement::SetTransaction { .. }
        | Statement::SetNames { .. } => "SET".to_string(),
        Statement::Pragma { .. } => "PRAGMA".to_string(),
        Statement::Call(_) => "CALL".to_string(),
        Statement::Directory { .. } | Statement::LoadData { .. } => "file load".to_string(),
        _ => "this statement".to_string(),
    }
}

/// The query a statement executes, for row-cap injection. `allowed()` has
/// already narrowed this to `Query` and `Explain(Query)`; capping the inner
/// query of `EXPLAIN ANALYZE` matters because that form *executes* the plan.
fn top_query(statement: &mut Statement) -> Option<&mut Query> {
    match statement {
        Statement::Query(query) => Some(query),
        Statement::Explain { statement, .. } => match statement.as_mut() {
            Statement::Query(query) => Some(query),
            _ => None,
        },
        _ => None,
    }
}

struct Guard {
    denied: Option<ScratchRejection>,
}

impl Visitor for Guard {
    type Break = ();

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        if let Expr::Function(function) = expr
            && let Some(denied) = denied_file_name(&function.name)
        {
            self.denied = Some(denied);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }

    fn pre_visit_table_factor(&mut self, factor: &TableFactor) -> ControlFlow<Self::Break> {
        // A table function in `FROM` (`read_csv('x.csv')`) carries its name in
        // `Table { args }`; a `LATERAL fn(...)` in `Function`. Plain relations
        // get the same per-part check — stricter than the connector's
        // whole-name posture, and safe here because every relation inside
        // scratch is one the model itself created. Derived tables, UNNEST and
        // JSON_TABLE carry no name of their own; the visitor recurses into
        // their subqueries and expressions, which these checks cover.
        let (TableFactor::Table { name, .. } | TableFactor::Function { name, .. }) = factor else {
            return ControlFlow::Continue(());
        };
        if let Some(denied) = denied_file_name(name) {
            self.denied = Some(denied);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    }
}

/// `Some(refusal)` when any identifier part of the name is a file reader.
/// A single-quoted identifier is DuckDB's file-read shorthand (`FROM 'data.csv'`)
/// and is refused as the file path it is.
fn denied_file_name(name: &ObjectName) -> Option<ScratchRejection> {
    name.0.iter().find_map(|ident| {
        if ident.quote_style == Some('\'') {
            return Some(ScratchRejection::FilePathRelation(ident.value.clone()));
        }
        let part = ident.value.to_ascii_lowercase();
        if FILE_READ_PREFIXES
            .iter()
            .any(|prefix| part.starts_with(prefix))
            || FILE_READ_FUNCTIONS.contains(&part.as_str())
        {
            return Some(ScratchRejection::FileFunction(part));
        }
        None
    })
}

/// Injects the row cap into a read the way the connector's gate does: an
/// explicit bound at or under the cap is left as written; anything looser is
/// replaced by `LIMIT cap+1` (any FETCH dropped) so the collection layer can
/// report `truncated` instead of silently cutting.
fn cap(query: &mut Query, max_rows: usize) {
    let limit = literal(query.limit.as_ref());
    let fetch = query
        .fetch
        .as_ref()
        .and_then(|fetch| literal(fetch.quantity.as_ref()));
    if limit.or(fetch).is_some_and(|bound| bound <= max_rows) {
        return;
    }
    query.limit = Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(
        max_rows.saturating_add(1).to_string(),
        false,
    )));
    query.fetch = None;
}

fn literal(limit: Option<&sqlparser::ast::Expr>) -> Option<usize> {
    match limit {
        Some(sqlparser::ast::Expr::Value(sqlparser::ast::Value::Number(value, _))) => {
            value.parse().ok()
        }
        _ => None,
    }
}
