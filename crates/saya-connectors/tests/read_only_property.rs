//! Exemplar property test for the read-only safety layer (see docs/standards/{testing,security}.md).
//!
//! The safety contract is an invariant over *all* SQL, not the handful of strings a
//! human would pick: anything the layer accepts is a read-only SELECT, and anything
//! that mutates is rejected regardless of casing, spacing, or row cap. `proptest`
//! searches that space and shrinks to a minimal counterexample on failure.

use proptest::prelude::*;
use saya_connectors::{prepare_postgres_sql, prepare_sqlite_sql};

/// A lowercase SQL identifier safe to interpolate into a statement.
fn ident() -> impl Strategy<Value = String> {
    "[a-z][a-z0-9_]{0,10}".prop_filter("avoid SQL keywords", |s| {
        !matches!(s.as_str(), "select" | "from" | "where" | "limit" | "table")
    })
}

proptest! {
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
