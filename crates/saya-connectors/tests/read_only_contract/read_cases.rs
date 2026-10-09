use super::*;

#[test]
fn safety_rejects_row_locking_clauses() {
    for sql in [
        "SELECT * FROM orders FOR UPDATE",
        "SELECT * FROM orders FOR SHARE",
        "WITH x AS (SELECT * FROM orders FOR UPDATE) SELECT * FROM x",
    ] {
        assert!(prepare_postgres_sql(sql, 10).is_err(), "must reject {sql}");
    }
}

/// The denylist must reach a denied function name no matter where it sits in the
/// tree — in `FROM` as a table function, behind `LATERAL`, schema-qualified, or
/// inside a derived table / scalar subquery — and the lock check must reach
/// every `Query` node, not just the top level. Each string below was previously
/// allowed and must now be rejected.
#[test]
fn safety_rejects_denied_functions_and_locks_in_every_position() {
    let cases: &[(&str, &str)] = &[
        ("pg", "SELECT * FROM pg_catalog.pg_read_file('/etc/passwd')"),
        // Both forms of the same name: the qualified one was the actual
        // bypass, the bare one was already caught by the whole-name relation
        // check and must stay caught.
        ("pg", "SELECT * FROM pg_catalog.pg_advisory_lock(1)"),
        ("pg", "SELECT * FROM pg_advisory_lock(1)"),
        ("pg", "SELECT * FROM pg_catalog.pg_sleep(100)"),
        ("pg", "SELECT * FROM t, LATERAL pg_read_file('/x')"),
        ("duck", "SELECT * FROM main.read_csv('/etc/passwd')"),
        ("mysql", "SELECT * FROM x.load_file('/etc/passwd')"),
        ("snow", "SELECT * FROM d.s.directory(@x)"),
        ("pg", "SELECT * FROM (SELECT 1 FROM t FOR UPDATE) s"),
        ("pg", "SELECT * FROM (SELECT 1 FROM t FOR SHARE) s"),
        // A locking clause inside a scalar subquery expression.
        ("pg", "SELECT (SELECT 1 FROM t FOR UPDATE)"),
        // A denied name reached through a CTE or a nested
        // subquery's `FROM` is denied identically to the top-level case.
        (
            "pg",
            "WITH x AS (SELECT * FROM pg_catalog.pg_read_file('/etc/passwd')) SELECT * FROM x",
        ),
        (
            "pg",
            "SELECT * FROM (SELECT * FROM pg_catalog.pg_read_file('/etc/passwd')) s",
        ),
    ];
    for (backend, sql) in cases {
        let result = match *backend {
            "pg" => prepare_postgres_sql(sql, 10),
            "duck" => prepare_duckdb_sql(sql, 10),
            "mysql" => prepare_mysql_sql(sql, 10),
            "snow" => prepare_snowflake_sql(sql, 10),
            _ => unreachable!(),
        };
        assert!(result.is_err(), "must reject ({backend}) {sql}");
    }
}

/// Tightening only: the ordinary read-only statements that were already allowed
/// stay allowed, and `FETCH FIRST … ROWS ONLY` keeps its clause (the cap does
/// not collapse it into a `LIMIT` when the bound is already within the cap).
#[test]
fn safety_keeps_ordinary_reads_allowed() {
    for sql in [
        "SELECT * FROM orders",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "SELECT 1 FROM a UNION ALL SELECT 2 FROM b",
        "EXPLAIN ANALYZE SELECT 1 FROM t",
        "VALUES (1),(2)",
        "SELECT 1 FROM t FETCH FIRST 5 ROWS ONLY",
    ] {
        assert!(prepare_postgres_sql(sql, 10).is_ok(), "must accept {sql}");
    }

    // With a cap of 10 the FETCH bound of 5 is already within the cap, so the
    // cap must leave the clause untouched (Postgres rejects LIMIT+FETCH
    // together, so a FETCH within the cap is never rewritten to a LIMIT).
    let prepared = prepare_postgres_sql("SELECT 1 FROM t FETCH FIRST 5 ROWS ONLY", 10)
        .expect("FETCH FIRST must stay accepted");
    assert!(
        prepared.to_uppercase().contains("FETCH FIRST 5 ROWS ONLY"),
        "FETCH clause must be preserved: {prepared}"
    );
}

#[test]
fn safety_fetch_first_queries_stay_valid_and_bounded() {
    let prepared = prepare_postgres_sql("SELECT * FROM events FETCH FIRST 5 ROWS ONLY", 1)
        .expect("FETCH FIRST query must be accepted");
    let upper = prepared.to_uppercase();
    assert!(
        !(upper.contains("LIMIT") && upper.contains("FETCH")),
        "Postgres rejects LIMIT together with FETCH; prepared was: {prepared}"
    );
}

#[test]
fn safety_rejections_name_the_reason_and_next_step() {
    let cases = [
        ("DELETE FROM orders", "DELETE"),
        ("SELECT 1; SELECT 2", "one statement"),
        ("SELECT * INTO archive FROM orders", "SELECT INTO"),
        (
            "WITH x AS (INSERT INTO t VALUES (1)) SELECT * FROM x",
            "INSERT",
        ),
        ("SELECT nextval('seq')", "nextval"),
        ("SELECT * FROM orders FOR UPDATE", "row locks"),
        ("SELECT 1 FROM", "read-only statement"),
    ];
    for (sql, expected) in cases {
        let error = prepare_postgres_sql(sql, 10)
            .expect_err("must reject")
            .to_string();
        assert!(
            error.contains("rejected") && error.contains(expected),
            "rejection must explain itself ({expected}): {error}"
        );
    }
    assert!(
        prepare_postgres_sql("SELECT 1", 0).is_err(),
        "zero cap still rejected"
    );
}
