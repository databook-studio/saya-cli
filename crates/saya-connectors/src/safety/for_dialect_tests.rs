//! Tests for `prepare_for_dialect`, the dialect-generic entry to the
//! execution-time read-only gate.

use saya_types::{ConnectionError, SqlDialect};

use crate::{
    prepare_bigquery_sql, prepare_clickhouse_sql, prepare_duckdb_sql, prepare_for_dialect,
    prepare_mysql_sql, prepare_postgres_sql, prepare_snowflake_sql, prepare_sqlite_sql,
};

const ROWS: usize = 10;

type Wrapper = fn(&str, usize) -> Result<String, ConnectionError>;

/// Every wired dialect paired with the exact wrapper the connector calls at
/// execution time, so parity is checked against the real gate.
const CASES: &[(SqlDialect, Wrapper)] = &[
    (SqlDialect::Postgres, prepare_postgres_sql),
    (SqlDialect::Mysql, prepare_mysql_sql),
    (SqlDialect::DuckDb, prepare_duckdb_sql),
    (SqlDialect::Snowflake, prepare_snowflake_sql),
    (SqlDialect::Sqlite, prepare_sqlite_sql),
    (SqlDialect::ClickHouse, prepare_clickhouse_sql),
    (SqlDialect::BigQuery, prepare_bigquery_sql),
];

/// The generic dispatch and the per-dialect wrapper must produce the identical
/// outcome — the same prepared string when allowed, the same refusal when not.
fn assert_same_outcome(sql: &str, rows: usize, dialect: SqlDialect, wrapper: Wrapper) {
    let generic = prepare_for_dialect(sql, rows, dialect).map_err(|e| e.to_string());
    let direct = wrapper(sql, rows).map_err(|e| e.to_string());
    assert_eq!(
        generic, direct,
        "{dialect:?}: dispatch must match its wrapper for {sql:?}"
    );
}

fn assert_allowed(sql: &str, rows: usize, dialect: SqlDialect, wrapper: Wrapper) {
    assert!(
        wrapper(sql, rows).is_ok(),
        "{dialect:?} must accept {sql:?} at a row cap of {rows}"
    );
    assert_same_outcome(sql, rows, dialect, wrapper);
}

fn assert_refused(sql: &str, rows: usize, dialect: SqlDialect, wrapper: Wrapper) {
    assert!(
        wrapper(sql, rows).is_err(),
        "{dialect:?} must refuse {sql:?}"
    );
    assert_same_outcome(sql, rows, dialect, wrapper);
}

#[test]
fn select_one_is_prepared_identically_for_every_dialect() {
    for (dialect, wrapper) in CASES {
        assert_allowed("SELECT 1", ROWS, *dialect, *wrapper);
    }
}

#[test]
fn delete_is_refused_for_every_dialect() {
    for (dialect, wrapper) in CASES {
        assert_refused("DELETE FROM t", ROWS, *dialect, *wrapper);
    }
}

#[test]
fn drop_table_is_refused_for_every_dialect() {
    for (dialect, wrapper) in CASES {
        assert_refused("DROP TABLE t", ROWS, *dialect, *wrapper);
    }
}

#[test]
fn multiple_statements_are_refused_for_every_dialect() {
    for (dialect, wrapper) in CASES {
        assert_refused("SELECT 1; SELECT 2", ROWS, *dialect, *wrapper);
    }
}

#[test]
fn zero_row_cap_is_refused_for_every_dialect() {
    for (dialect, wrapper) in CASES {
        assert_refused("SELECT 1", 0, *dialect, *wrapper);
    }
}

/// Each statement here is denied by exactly one backend's policy while the
/// Postgres wrapper allows it (see `read_only.rs`'s own tests), so a mis-route
/// — e.g. falling back to the Postgres policy the way `join_check` does —
/// shows up as a parity failure.
#[test]
fn per_backend_policy_denials_route_to_the_right_wrapper() {
    assert_refused(
        "SELECT load_extension('x')",
        ROWS,
        SqlDialect::Sqlite,
        prepare_sqlite_sql,
    );
    assert_refused(
        "SELECT * FROM read_csv('x')",
        ROWS,
        SqlDialect::DuckDb,
        prepare_duckdb_sql,
    );
    assert_refused(
        "SELECT system$type('x')",
        ROWS,
        SqlDialect::Snowflake,
        prepare_snowflake_sql,
    );
    assert_allowed(
        "SELECT load_extension('x')",
        ROWS,
        SqlDialect::Postgres,
        prepare_postgres_sql,
    );
}
