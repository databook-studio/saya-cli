use saya_connectors::{
    prepare_duckdb_sql, prepare_mysql_sql, prepare_postgres_sql, prepare_snowflake_sql,
};

#[test]
fn safety_rejects_zero_caps_writes_and_duckdb_administration() {
    for sql in [
        "SELECT 1; SELECT 2",
        "SELECT * INTO archive FROM events",
        "WITH x AS (DELETE FROM events RETURNING *) SELECT * FROM x",
        "ATTACH 'other.db' AS other",
        "COPY events TO 'out.csv'",
        "INSTALL httpfs",
        "LOAD httpfs",
        "SET threads = 1",
        "PRAGMA enable_external_access",
    ] {
        assert!(prepare_postgres_sql(sql, 0).is_err(), "must reject {sql}");
        assert!(prepare_postgres_sql(sql, 10).is_err(), "must reject {sql}");
    }
}

#[test]
fn safety_keeps_values_and_newest_limit_semantics() {
    assert!(prepare_postgres_sql("VALUES (1), (2)", 1).is_ok());
    assert!(prepare_postgres_sql("WITH x AS (VALUES (1)) SELECT * FROM x", 1).is_ok());
    assert!(
        prepare_postgres_sql("SELECT 1 LIMIT 99", 1)
            .unwrap()
            .contains("LIMIT 2")
    );
}

#[test]
fn safety_does_not_scan_literals_or_identifiers_as_keywords() {
    assert!(prepare_postgres_sql("SELECT 'COPY LOAD' AS copy_load", 1).is_ok());
    assert!(prepare_postgres_sql("SELECT copy_load FROM report", 1).is_ok());
}

#[test]
fn safety_rejects_mutating_and_external_functions_from_the_ast() {
    for sql in ["SELECT nextval('id_seq')", "SELECT setval('id_seq', 9)"] {
        assert!(prepare_postgres_sql(sql, 1).is_err(), "must reject {sql}");
    }

    for sql in [
        "SELECT * FROM read_csv('input.csv')",
        "SELECT * FROM read_json('input.json')",
        "SELECT * FROM sqlite_scan('other.db', 'events')",
    ] {
        assert!(prepare_duckdb_sql(sql, 1).is_err(), "must reject {sql}");
        assert!(prepare_postgres_sql(sql, 1).is_ok(), "must accept {sql}");
    }
}

#[test]
fn safety_rejects_schema_qualified_and_requoted_denied_functions() {
    // Qualification or requoting must never bypass the function guard.
    for sql in [
        "SELECT pg_catalog.nextval('seq')",
        "SELECT public.setval('seq', 9)",
        "SELECT \"nextval\"('seq')",
    ] {
        assert!(prepare_postgres_sql(sql, 10).is_err(), "must reject {sql}");
    }

    for sql in ["SELECT `nextval`('seq')", "SELECT mysql.`setval`('seq', 9)"] {
        assert!(prepare_mysql_sql(sql, 10).is_err(), "must reject {sql}");
    }

    assert!(
        prepare_duckdb_sql("SELECT main.read_csv('input.csv')", 10).is_err(),
        "schema-qualified read_csv must be rejected"
    );
    assert!(
        prepare_snowflake_sql(
            "SELECT my_db.my_schema.build_scoped_file_url('stage', 'f')",
            10
        )
        .is_err(),
        "schema-qualified build_scoped_file_url must be rejected"
    );
    assert!(
        prepare_snowflake_sql(
            "SELECT my_db.my_schema.system$get_presigned_url('stage', 'f')",
            10
        )
        .is_err(),
        "prefix rule must apply to the function name part, not the joined path"
    );

    // A table literally named like a denied function stays blocked too
    // (relations keep the stricter whole-name check; fail-closed by design).
    assert!(prepare_postgres_sql("SELECT * FROM nextval", 10).is_err());
    assert!(prepare_postgres_sql("SELECT nextval FROM seq_state", 10).is_ok());
}

#[test]
fn safety_rejects_session_and_server_mutating_functions_on_postgres() {
    for sql in [
        "SELECT set_config('role', 'admin', false)",
        "SELECT setseed(0.5)",
        "SELECT pg_advisory_lock(42)",
        "SELECT pg_advisory_xact_lock(42)",
        "SELECT pg_advisory_unlock(42)",
        "SELECT pg_terminate_backend(pid)",
        "SELECT pg_cancel_backend(pid)",
        "SELECT pg_reload_conf()",
        "SELECT pg_read_file('/etc/passwd')",
        "SELECT pg_read_binary_file('/etc/passwd')",
        "SELECT pg_ls_dir('/')",
        "SELECT lo_import('/etc/passwd')",
        "SELECT lo_export(lo, '/tmp/out')",
        "SELECT dblink_exec('dbname=x port=5432', 'INSERT INTO t VALUES (1)')",
        "SELECT pg_sleep(1000)",
        "SELECT pg_sleep_for(interval '1 hour')",
        "SELECT pg_create_restore_point('x')",
        "SELECT pg_logical_emit_message(true, 'x', 'y')",
    ] {
        assert!(prepare_postgres_sql(sql, 10).is_err(), "must reject {sql}");
    }

    // Ordinary scalar functions stay available.
    assert!(prepare_postgres_sql("SELECT count(*) FROM t", 10).is_ok());
    assert!(prepare_postgres_sql("SELECT current_setting('work_mem')", 10).is_ok());
}

#[test]
fn safety_rejects_locking_and_file_functions_on_mysql() {
    for sql in [
        "SELECT get_lock('m', 10)",
        "SELECT release_lock('m')",
        "SELECT release_all_locks()",
        "SELECT load_file('/etc/passwd')",
        "SELECT sleep(1000)",
        "SELECT sys_exec('id')",
        "SELECT sys_eval('id')",
    ] {
        assert!(prepare_mysql_sql(sql, 10).is_err(), "must reject {sql}");
    }

    assert!(prepare_mysql_sql("SELECT 1 + 2", 10).is_ok());
}

#[test]
fn safety_caps_explain_like_plain_selects() {
    let analyzed = prepare_postgres_sql("EXPLAIN ANALYZE SELECT * FROM events LIMIT 99", 1)
        .expect("EXPLAIN ANALYZE of a plain SELECT must be accepted");
    assert!(
        analyzed.to_uppercase().contains("LIMIT 2"),
        "EXPLAIN ANALYZE must carry the row cap: {analyzed}"
    );

    let plain = prepare_postgres_sql("EXPLAIN SELECT * FROM events", 5)
        .expect("plain EXPLAIN must stay accepted");
    assert!(
        plain.to_uppercase().contains("LIMIT 6"),
        "EXPLAIN must carry the row cap: {plain}"
    );
}

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
        // Q3: a locking clause inside a scalar subquery expression.
        ("pg", "SELECT (SELECT 1 FROM t FOR UPDATE)"),
        // Invariant 1: a denied name reached through a CTE or a nested
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
