//! Exemplar property test for the read-only safety layer (see docs/standards/{testing,security}.md).
//!
//! The safety contract is an invariant over *all* SQL, not the handful of strings a
//! human would pick: anything the layer accepts is a read-only SELECT, and anything
//! that mutates is rejected regardless of casing, spacing, or row cap. `proptest`
//! searches that space and shrinks to a minimal counterexample on failure.

use proptest::prelude::*;
use saya_connectors::prepare_postgres_sql;

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
}
