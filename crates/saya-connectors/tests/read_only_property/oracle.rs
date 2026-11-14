use proptest::prelude::*;
use sqlparser::{ast::Statement, dialect::Dialect, parser::Parser};

/// An independent parser-based oracle that parses `sql` with `dialect` and returns `true`
/// if and only if `sql` contains exactly one statement and that statement belongs to a
/// read-only statement class (`Statement::Query`, `Statement::Explain` wrapping a `Query`,
/// or a `Show*` variant). Everything else returns `false`.
pub(super) fn is_read_only_statement_class(sql: &str, dialect: &dyn Dialect) -> bool {
    let Ok(statements) = Parser::parse_sql(dialect, sql) else {
        return false;
    };
    if statements.len() != 1 {
        return false;
    }
    match &statements[0] {
        Statement::Query(_) => true,
        Statement::Explain { statement, .. } => matches!(statement.as_ref(), Statement::Query(_)),
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
        | Statement::ShowCollation { .. } => true,
        _ => false,
    }
}

/// A lowercase SQL identifier safe to interpolate into a statement. The
/// `i_` prefix guarantees it can never collide with a reserved word (a bare
/// `[a-z][a-z0-9_]*` draw once produced `all`, which parses as a SELECT
/// quantifier and made acceptance assertions flaky).
pub(super) fn ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}".prop_map(|base| format!("i_{base}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TemplateKind {
    ReadOnly,
    NonReadOnlyPostgres,
    NonReadOnlySqlite,
    NonReadOnlyBoth,
}

pub(super) fn sql_corpus_strategy() -> impl Strategy<Value = (String, TemplateKind)> {
    (
        ident(),
        ident(),
        ident(),
        1usize..10000,
        1usize..10000,
        any::<bool>(),
    )
        .prop_flat_map(|(col1, col2, tbl, n1, n2, uppercase)| {
            let templates = vec![
                // Read-only forms
                (format!("SELECT {col1} FROM {tbl}"), TemplateKind::ReadOnly),
                (
                    format!("SELECT {col1}, {col2} FROM {tbl} WHERE {col1} = {n1}"),
                    TemplateKind::ReadOnly,
                ),
                (
                    format!("SELECT count({col1}), max({col2}) FROM {tbl}"),
                    TemplateKind::ReadOnly,
                ),
                (
                    format!("SELECT abs({n1}), upper({col1}) FROM {tbl}"),
                    TemplateKind::ReadOnly,
                ),
                (format!("VALUES ({n1}), ({n2})"), TemplateKind::ReadOnly),
                (
                    format!("WITH cte AS (SELECT {col1} FROM {tbl}) SELECT * FROM cte"),
                    TemplateKind::ReadOnly,
                ),
                (
                    format!("EXPLAIN SELECT {col1} FROM {tbl}"),
                    TemplateKind::ReadOnly,
                ),
                (
                    format!("EXPLAIN ANALYZE SELECT * FROM {tbl} WHERE {col1} = {n1}"),
                    TemplateKind::ReadOnly,
                ),
                ("SHOW TABLES".to_string(), TemplateKind::ReadOnly),
                (format!("SHOW COLUMNS FROM {tbl}"), TemplateKind::ReadOnly),
                ("SHOW VARIABLES".to_string(), TemplateKind::ReadOnly),
                // Non-read-only forms (both backends)
                (
                    format!("INSERT INTO {tbl} VALUES ({n1})"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    format!("INSERT INTO {tbl} ({col1}) VALUES ({n1})"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    format!("UPDATE {tbl} SET {col1} = {n1}"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (format!("DELETE FROM {tbl}"), TemplateKind::NonReadOnlyBoth),
                (
                    format!("DELETE FROM {tbl} WHERE {col1} = {n1}"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    format!("CREATE TABLE {tbl} ({col1} INT)"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (format!("DROP TABLE {tbl}"), TemplateKind::NonReadOnlyBoth),
                (
                    format!("ALTER TABLE {tbl} ADD COLUMN {col1} INT"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (format!("TRUNCATE {tbl}"), TemplateKind::NonReadOnlyBoth),
                (
                    format!("TRUNCATE TABLE {tbl}"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    format!("COPY {tbl} FROM 'data.csv'"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "PRAGMA user_version".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "PRAGMA foreign_keys = ON".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "SET timezone = 'UTC'".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "ATTACH DATABASE 'foo.db' AS aux".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "SELECT 1; SELECT 2".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    format!("SELECT {col1} FROM {tbl}; DELETE FROM {tbl}"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "SELECT nextval('seq')".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "SELECT setval('seq', 1)".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "SELECT pg_catalog.nextval('seq')".to_string(),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    format!("SELECT * FROM {tbl} FOR UPDATE"),
                    TemplateKind::NonReadOnlyBoth,
                ),
                (
                    "SELECT set_config('role', 'admin', false)".to_string(),
                    TemplateKind::NonReadOnlyPostgres,
                ),
                (
                    "SELECT pg_advisory_lock(42)".to_string(),
                    TemplateKind::NonReadOnlyPostgres,
                ),
                (
                    "SELECT pg_sleep(10)".to_string(),
                    TemplateKind::NonReadOnlyPostgres,
                ),
                // SQLite specific denied functions
                (
                    "SELECT load_extension('x')".to_string(),
                    TemplateKind::NonReadOnlySqlite,
                ),
                (
                    "SELECT readfile('x')".to_string(),
                    TemplateKind::NonReadOnlySqlite,
                ),
                (
                    "SELECT writefile('a', 'b')".to_string(),
                    TemplateKind::NonReadOnlySqlite,
                ),
            ];

            prop::sample::select(templates).prop_map(move |(stmt, kind)| {
                let stmt = if uppercase {
                    stmt.to_uppercase()
                } else {
                    stmt.to_lowercase()
                };
                (stmt, kind)
            })
        })
}
