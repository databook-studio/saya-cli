use saya_connectors::prepare_postgres_sql;

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
    for sql in [
        "SELECT nextval('id_seq')",
        "SELECT setval('id_seq', 9)",
        "SELECT * FROM read_csv('input.csv')",
        "SELECT * FROM read_json('input.json')",
        "SELECT * FROM sqlite_scan('other.db', 'events')",
    ] {
        assert!(prepare_postgres_sql(sql, 1).is_err(), "must reject {sql}");
    }
}
