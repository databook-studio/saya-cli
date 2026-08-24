//! Exemplar property test for the read-only safety layer.
//!
//! Independent oracle statement-class verification:
//! This test suite proves that any SQL statement accepted by `prepare_postgres_sql` or
//! `prepare_sqlite_sql` is converted into a string that belongs strictly to a read-only
//! statement class (`Statement::Query`, `Explain` wrapping a `Query`, or a `Show*` variant),
//! verified via an independent parser-based oracle implemented in this test module.
//!
//! LIMIT: This oracle proves statement-CLASS read-only safety. It does not prove that an
//! accepted function (e.g. custom UDF) is side-effect free; function-level side-effects
//! are enforced at the DB session level and via read-only connection roles (see SECURITY.md).

use proptest::prelude::*;
use saya_connectors::{prepare_postgres_sql, prepare_sqlite_sql};
use sqlparser::{
    ast::Statement,
    dialect::{Dialect, PostgreSqlDialect, SQLiteDialect},
    parser::Parser,
};

/// An independent parser-based oracle that parses `sql` with `dialect` and returns `true`
/// if and only if `sql` contains exactly one statement and that statement belongs to a
/// read-only statement class (`Statement::Query`, `Statement::Explain` wrapping a `Query`,
/// or a `Show*` variant). Everything else returns `false`.
fn is_read_only_statement_class(sql: &str, dialect: &dyn Dialect) -> bool {
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

/// A lowercase SQL identifier safe to interpolate into a statement.
fn ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}".prop_filter("avoid SQL keywords", |s| {
        !matches!(
            s.as_str(),
            "select" | "from" | "where" | "limit" | "table" | "into" | "values" | "set" | "with"
        )
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemplateKind {
    ReadOnly,
    NonReadOnlyPostgres,
    NonReadOnlySqlite,
    NonReadOnlyBoth,
}

fn sql_corpus_strategy() -> impl Strategy<Value = (String, TemplateKind)> {
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

proptest! {
    /// Invariant: Any statement accepted by `prepare_postgres_sql` or `prepare_sqlite_sql`
    /// MUST produce a prepared SQL string that the independent oracle classifies as a
    /// read-only statement class. Also asserts that known non-read-only templates are rejected.
    #[test]
    fn independent_oracle_proves_read_only_statement_class(
        (sql, kind) in sql_corpus_strategy(),
        cap in 1usize..100,
    ) {
        let pg_dialect = PostgreSqlDialect {};
        if let Ok(prepared) = prepare_postgres_sql(&sql, cap) {
            prop_assert!(
                is_read_only_statement_class(&prepared, &pg_dialect),
                "Postgres safety layer accepted statement that independent oracle did not classify as read-only class: input={sql}, prepared={prepared}"
            );
        }

        let sqlite_dialect = SQLiteDialect {};
        if let Ok(prepared) = prepare_sqlite_sql(&sql, cap) {
            prop_assert!(
                is_read_only_statement_class(&prepared, &sqlite_dialect),
                "SQLite safety layer accepted statement that independent oracle did not classify as read-only class: input={sql}, prepared={prepared}"
            );
        }

        if kind == TemplateKind::NonReadOnlyBoth || kind == TemplateKind::NonReadOnlyPostgres {
            prop_assert!(
                prepare_postgres_sql(&sql, cap).is_err(),
                "Postgres safety layer accepted non-read-only template: {sql}"
            );
        }
        if kind == TemplateKind::NonReadOnlyBoth || kind == TemplateKind::NonReadOnlySqlite {
            prop_assert!(
                prepare_sqlite_sql(&sql, cap).is_err(),
                "SQLite safety layer accepted non-read-only template: {sql}"
            );
        }
    }

    /// Any single SELECT over a simple table is accepted, stays a SELECT, and is
    /// row-capped — the layer never turns an accepted read into something else.
    #[test]
    fn selects_are_accepted_and_capped(
        col in ident(),
        tbl in ident(),
        limit in 1usize..1000,
        cap in 1usize..100,
    ) {
        let sql = format!("SELECT {col} FROM {tbl} LIMIT {limit}");
        let prepared = prepare_postgres_sql(&sql, cap).expect("a plain SELECT must be accepted");
        let upper = prepared.to_uppercase();
        prop_assert!(upper.trim_start().starts_with("SELECT"), "accepted output is not a SELECT: {prepared}");
        prop_assert!(upper.contains("LIMIT"), "accepted SELECT lost its row cap: {prepared}");
        prop_assert!(
            is_read_only_statement_class(&prepared, &PostgreSqlDialect {}),
            "prepared statement must be classified as read-only by oracle: {prepared}"
        );
    }

    /// Mutating statements are rejected regardless of casing, whitespace, or row cap.
    #[test]
    fn mutations_are_always_rejected(
        tbl in ident(),
        cap in 0usize..100,
        uppercase in any::<bool>(),
    ) {
        for stmt in [
            format!("DELETE FROM {tbl}"),
            format!("UPDATE {tbl} SET x = 1"),
            format!("INSERT INTO {tbl} VALUES (1)"),
            format!("DROP TABLE {tbl}"),
            format!("TRUNCATE {tbl}"),
        ] {
            let stmt = if uppercase { stmt.to_uppercase() } else { stmt.to_lowercase() };
            prop_assert!(
                prepare_postgres_sql(&stmt, cap).is_err(),
                "read-only layer accepted a mutation: {stmt}"
            );
        }
    }

    /// SQLite: SELECT, VALUES, and WITH CTE SELECT are accepted and row-capped.
    #[test]
    fn sqlite_selects_and_values_accepted_and_capped(
        col in ident(),
        tbl in ident(),
        cap in 1usize..100,
    ) {
        let stmts = [
            format!("SELECT {col} FROM {tbl}"),
            "VALUES (1), (2)".to_string(),
            format!("WITH cte AS (SELECT {col} FROM {tbl}) SELECT * FROM cte"),
        ];
        for stmt in stmts {
            let res = prepare_sqlite_sql(&stmt, cap);
            prop_assert!(res.is_ok(), "SQLite safety layer rejected valid query: {stmt}");
            let prepared = res.unwrap();
            prop_assert!(prepared.to_uppercase().contains("LIMIT"), "prepared query lost LIMIT: {prepared}");
            prop_assert!(
                is_read_only_statement_class(&prepared, &SQLiteDialect {}),
                "prepared statement must be classified as read-only by oracle: {prepared}"
            );
        }
    }

    /// SQLite: Mutating, administrative, or multi-statements are rejected.
    #[test]
    fn sqlite_mutations_and_unauthorized_rejected(
        tbl in ident(),
        cap in 0usize..100,
        uppercase in any::<bool>(),
    ) {
        let stmts = [
            format!("DELETE FROM {tbl}"),
            format!("UPDATE {tbl} SET x = 1"),
            format!("INSERT INTO {tbl} VALUES (1)"),
            format!("DROP TABLE {tbl}"),
            format!("CREATE TABLE {tbl} (x INT)"),
            "ATTACH DATABASE 'foo.db' AS aux".to_string(),
            "PRAGMA user_version".to_string(),
            "SELECT 1; SELECT 2".to_string(),
        ];
        for stmt in stmts {
            let stmt_str = if uppercase { stmt.to_uppercase() } else { stmt.to_lowercase() };
            prop_assert!(
                prepare_sqlite_sql(&stmt_str, cap).is_err(),
                "SQLite safety layer accepted unauthorized statement: {stmt_str}"
            );
        }
    }

    /// SQLite: Denied functions load_extension, readfile, writefile are rejected.
    #[test]
    fn sqlite_denied_functions_rejected(
        cap in 1usize..100,
    ) {
        let stmts = [
            "SELECT load_extension('x')",
            "SELECT readfile('x')",
            "SELECT writefile('a', 'b')",
        ];
        for stmt in stmts {
            prop_assert!(
                prepare_sqlite_sql(stmt, cap).is_err(),
                "SQLite safety layer accepted denied function: {stmt}"
            );
        }
    }
}

#[cfg(test)]
mod oracle_tests {
    use super::*;

    #[test]
    fn test_independent_oracle_classification() {
        let pg = PostgreSqlDialect {};
        let sqlite = SQLiteDialect {};

        // Positive read-only cases
        assert!(is_read_only_statement_class("SELECT 1", &pg));
        assert!(is_read_only_statement_class(
            "SELECT a FROM b WHERE c = 1",
            &sqlite
        ));
        assert!(is_read_only_statement_class("VALUES (1), (2)", &pg));
        assert!(is_read_only_statement_class(
            "WITH cte AS (SELECT 1) SELECT * FROM cte",
            &sqlite
        ));
        assert!(is_read_only_statement_class("EXPLAIN SELECT 1", &pg));
        assert!(is_read_only_statement_class("SHOW TABLES", &pg));

        // Negative non-read-only cases
        assert!(!is_read_only_statement_class(
            "INSERT INTO t VALUES (1)",
            &pg
        ));
        assert!(!is_read_only_statement_class("UPDATE t SET x = 1", &sqlite));
        assert!(!is_read_only_statement_class("DELETE FROM t", &pg));
        assert!(!is_read_only_statement_class("DROP TABLE t", &sqlite));
        assert!(!is_read_only_statement_class("CREATE TABLE t (x INT)", &pg));
        assert!(!is_read_only_statement_class(
            "ALTER TABLE t ADD COLUMN x INT",
            &sqlite
        ));
        assert!(!is_read_only_statement_class("TRUNCATE TABLE t", &pg));
        assert!(!is_read_only_statement_class("COPY t FROM 'f'", &pg));
        assert!(!is_read_only_statement_class(
            "PRAGMA user_version",
            &sqlite
        ));
        assert!(!is_read_only_statement_class("SET timezone = 'UTC'", &pg));
        assert!(!is_read_only_statement_class(
            "ATTACH DATABASE 'a' AS b",
            &sqlite
        ));
        assert!(!is_read_only_statement_class("EXPLAIN DELETE FROM t", &pg));

        // Multiple statements or invalid SQL
        assert!(!is_read_only_statement_class("SELECT 1; SELECT 2", &pg));
        assert!(!is_read_only_statement_class(
            "INVALID SQL STATEMENT",
            &sqlite
        ));
    }
}
