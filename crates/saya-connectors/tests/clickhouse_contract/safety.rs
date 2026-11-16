use saya_connectors::prepare_clickhouse_sql;

/// Destructive verbs the ClickHouse dialect admits — some parse to a statement
/// the allow-list rejects, others fail to parse at all. Both paths must end in
/// a rejection; the table below was built by probing each verb against the
/// sqlparser ClickHouse dialect and is re-checked here so a parser upgrade
/// cannot silently start accepting one.
const REJECTED_DESTRUCTIVE: &[&str] = &[
    "ALTER TABLE t DELETE WHERE x = 1",
    "ALTER TABLE t UPDATE x = 1 WHERE y = 2",
    "ALTER TABLE t ADD COLUMN c Int8",
    "ALTER TABLE t DROP COLUMN c",
    "ALTER TABLE t MODIFY COLUMN c String",
    "ALTER TABLE t RENAME COLUMN a TO b",
    "TRUNCATE TABLE t",
    "DROP TABLE t",
    "DROP DATABASE db",
    "CREATE TABLE t (x Int8)",
    "CREATE DATABASE db",
    "INSERT INTO t VALUES (1)",
    "INSERT INTO t SELECT * FROM s",
    "UPDATE t SET x = 1 WHERE y = 2",
    "DELETE FROM t WHERE x = 1",
    "OPTIMIZE TABLE t FINAL",
    "OPTIMIZE TABLE t",
    "SYSTEM RELOAD CONFIG",
    "SYSTEM FLUSH LOGS",
    "SYSTEM KILL QUERY",
    "KILL QUERY WHERE query_id = 'x'",
    "SET max_threads = 1",
    "DETACH TABLE t",
    "ATTACH TABLE t",
    "RENAME TABLE a TO b",
    "GRANT SELECT ON t TO u",
    "REVOKE SELECT ON t FROM u",
    "CHECK TABLE t",
];

/// Table functions that read from an arbitrary URL, object store, remote
/// server, the local filesystem, or another database. Each is an SSRF or
/// local-file-read vector that a read-only session must not open; the guard
/// reaches them wherever they appear (here, in `FROM` with call arguments).
const REJECTED_EXTERNAL: &[&str] = &[
    "SELECT * FROM url('http://x/y.csv', CSV)",
    "SELECT * FROM s3('bucket/key')",
    "SELECT * FROM remote('host:9000', 'db', 't')",
    "SELECT * FROM mysql('host:3306', 'db', 't')",
    "SELECT * FROM postgresql('host:5432', 'db', 't')",
    "SELECT * FROM file('data.csv')",
    "SELECT * FROM hdfs('uri')",
    "SELECT * FROM odbc('dsn')",
    "SELECT * FROM jdbc('url')",
];

#[test]
fn safety_rejects_every_clickhouse_destructive_verb() {
    for sql in REJECTED_DESTRUCTIVE {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_err(),
            "must reject destructive verb: {sql}"
        );
        // A zero row cap is rejected before the statement is even examined.
        assert!(
            prepare_clickhouse_sql(sql, 0).is_err(),
            "must reject zero-cap destructive verb: {sql}"
        );
    }
}

#[test]
fn safety_rejects_clickhouse_external_table_functions() {
    for sql in REJECTED_EXTERNAL {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_err(),
            "must reject external table function: {sql}"
        );
    }
    // A safe built-in table function stays available.
    assert!(prepare_clickhouse_sql("SELECT * FROM numbers(10)", 10).is_ok());
}

#[test]
fn safety_rejects_format_clause_so_the_connector_owns_wire_format() {
    for sql in [
        "SELECT * FROM t FORMAT JSONEachRow",
        "SELECT 1 FORMAT CSV",
        "EXPLAIN SELECT 1 FORMAT TSV",
    ] {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_err(),
            "must reject FORMAT clause: {sql}"
        );
    }
}

#[test]
fn safety_accepts_and_caps_clickhouse_reads() {
    for sql in [
        "SELECT * FROM t",
        "SELECT * FROM system.tables",
        "WITH x AS (SELECT 1) SELECT * FROM x",
        "SHOW TABLES",
        "EXPLAIN SELECT 1",
        "EXPLAIN ANALYZE SELECT 1",
    ] {
        assert!(
            prepare_clickhouse_sql(sql, 10).is_ok(),
            "must accept read: {sql}"
        );
    }
    // No explicit limit: the cap is injected one above the row cap, matching
    // the other connectors.
    assert!(
        prepare_clickhouse_sql("SELECT * FROM t", 10)
            .unwrap()
            .to_uppercase()
            .contains("LIMIT 11"),
    );
    // An explicit limit at or under the cap is left as written.
    assert!(
        prepare_clickhouse_sql("SELECT 1 FROM t LIMIT 5", 10)
            .unwrap()
            .to_uppercase()
            .contains("LIMIT 5"),
    );
}

#[test]
fn safety_rejections_name_the_reason() {
    let cases = [
        ("DELETE FROM t", "DELETE"),
        ("ALTER TABLE t ADD COLUMN c Int8", "ALTER TABLE"),
        ("OPTIMIZE TABLE t", "this statement"),
        ("SELECT * FROM url('http://x')", "url"),
        ("SELECT * FROM t FORMAT JSONEachRow", "FORMAT"),
    ];
    for (sql, expected) in cases {
        let error = prepare_clickhouse_sql(sql, 10)
            .expect_err("must reject")
            .to_string();
        assert!(
            error.contains("rejected") && error.contains(expected),
            "rejection must explain itself ({expected}): {error}"
        );
    }
}
