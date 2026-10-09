use super::*;
use proptest::prelude::*;

/// What the corpus entry claims about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Shape {
    /// A write on the scratch file itself — accepted by scratch, and still
    /// rejected by every user-DB `prepare_*`.
    Write,
    /// A read — accepted by scratch.
    Read,
    /// Refused by the validator: a file reader (with the sentinel's real path
    /// where the statement takes one).
    RefusedFile,
    /// Refused by the validator: the file/configuration/extension family.
    RefusedKind,
    /// Refused by the validator: more than one statement.
    RefusedMulti,
}

fn corpus() -> Vec<(String, Shape)> {
    let entries = vec![
        // Writes — the capability.
        (
            "CREATE TABLE t(a INTEGER, b VARCHAR)".to_string(),
            Shape::Write,
        ),
        (
            "INSERT INTO t VALUES (1, 'alpha')".to_string(),
            Shape::Write,
        ),
        ("UPDATE t SET a = 2".to_string(), Shape::Write),
        ("DELETE FROM t".to_string(), Shape::Write),
        (
            "ALTER TABLE t ADD COLUMN c INTEGER".to_string(),
            Shape::Write,
        ),
        ("DROP TABLE t".to_string(), Shape::Write),
        ("TRUNCATE t".to_string(), Shape::Write),
        ("CREATE VIEW v AS SELECT 1 AS x".to_string(), Shape::Write),
        ("DROP VIEW v".to_string(), Shape::Write),
        ("CREATE SCHEMA s".to_string(), Shape::Write),
        ("DROP SCHEMA s".to_string(), Shape::Write),
        ("CREATE INDEX idx ON t(a)".to_string(), Shape::Write),
        // A write that also hands rows back — still a write to the user gate.
        (
            "INSERT INTO t VALUES (1) RETURNING a".to_string(),
            Shape::Write,
        ),
        // Reads.
        ("SELECT 1".to_string(), Shape::Read),
        ("SELECT a, b FROM t ORDER BY a".to_string(), Shape::Read),
        ("SELECT count(*) FROM t".to_string(), Shape::Read),
        (
            "WITH c AS (SELECT 1 AS x) SELECT x FROM c".to_string(),
            Shape::Read,
        ),
        ("SELECT 1 UNION ALL SELECT 2".to_string(), Shape::Read),
        ("EXPLAIN SELECT a FROM t".to_string(), Shape::Read),
        ("EXPLAIN ANALYZE SELECT * FROM t".to_string(), Shape::Read),
        ("SHOW TABLES".to_string(), Shape::Read),
        // Refused: the file-reading family, by name.
        (
            "SELECT * FROM read_csv('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM read_csv_auto('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM read_parquet('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM read_json('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        ("SELECT read_text('{SENT}')".to_string(), Shape::RefusedFile),
        (
            "SELECT * FROM glob('*.csv')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM sqlite_scan('{SENT}', 't')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT * FROM parquet_scan('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "SELECT http_get('http://127.0.0.1:1/x')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "INSERT INTO t SELECT * FROM read_csv('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        (
            "CREATE TABLE c AS SELECT * FROM read_parquet('{SENT}')".to_string(),
            Shape::RefusedFile,
        ),
        // The file-read shorthand.
        ("SELECT * FROM '{SENT}'".to_string(), Shape::RefusedFile),
        // Refused statement kinds.
        ("INSTALL httpfs".to_string(), Shape::RefusedKind),
        ("LOAD httpfs".to_string(), Shape::RefusedKind),
        ("ATTACH '{SENT}' AS s".to_string(), Shape::RefusedKind),
        ("DETACH s".to_string(), Shape::RefusedKind),
        ("COPY t TO '{SENT}'".to_string(), Shape::RefusedKind),
        ("COPY t FROM '{SENT}'".to_string(), Shape::RefusedKind),
        (
            "PRAGMA enable_external_access".to_string(),
            Shape::RefusedKind,
        ),
        (
            "SET enable_external_access = true".to_string(),
            Shape::RefusedKind,
        ),
        ("SET threads = 2".to_string(), Shape::RefusedKind),
        ("CALL dbgen(sf = 1)".to_string(), Shape::RefusedKind),
        // Multi-statement.
        ("SELECT 1; SELECT 2".to_string(), Shape::RefusedMulti),
        (
            "CREATE TABLE u(a INTEGER); INSERT INTO u VALUES (1)".to_string(),
            Shape::RefusedMulti,
        ),
    ];
    entries
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Both directions at once. For every generated statement, executed
    /// through `scratch_sql` against a real run's scratch database:
    ///
    /// - accepted statements only ever touch the run dir — the sentinel
    ///   planted outside it keeps its exact bytes, and no accepted statement
    ///   ever trips DuckDB's permission layer;
    /// - every accepted *write* is still rejected by all seven user-DB
    ///   `prepare_*` functions, called exactly as they exist;
    /// - refused statements are refused by the validator with the typed
    ///   refusal their shape demands.
    #[test]
    fn every_accepted_statement_stays_inside_the_run_dir_and_every_write_is_rejected_by_the_user_gate(
        picked in proptest::collection::vec(proptest::sample::select(corpus()), 1..8),
    ) {
        let run = TempRun::new("battery");
        let sentinel = run.plant_sentinel();
        let tool = admitted(&run);
        for (template, shape) in picked {
            let sql = template.replace("{SENT}", &sentinel.display().to_string());
            match shape {
                Shape::Write => match run_sync(&tool, &sql) {
                    Ok(_) => {}
                    Err(ScratchError::Execution { message }) => prop_assert!(
                        !message.contains("Permission Error"),
                        "an accepted statement never trips the permission layer: {sql} — {message}"
                    ),
                    Err(other) => panic!("unexpected outcome for {sql}: {other}"),
                },
                Shape::Read => match run_sync(&tool, &sql) {
                    Ok(_) => {}
                    Err(ScratchError::Execution { message }) => prop_assert!(
                        !message.contains("Permission Error"),
                        "an accepted statement never trips the permission layer: {sql} — {message}"
                    ),
                    Err(other) => panic!("unexpected outcome for {sql}: {other}"),
                },
                Shape::RefusedFile => match run_sync(&tool, &sql) {
                    Err(ScratchError::Refused(ScratchRejection::FileFunction(_)))
                    | Err(ScratchError::Refused(ScratchRejection::FilePathRelation(_))) => {}
                    other => prop_assert!(
                        false,
                        "file reader must be refused by the validator: {other:?}"
                    ),
                },
                Shape::RefusedKind => match run_sync(&tool, &sql) {
                    Err(ScratchError::Refused(ScratchRejection::Statement(_))) => {}
                    other => prop_assert!(
                        false,
                        "statement family must be refused by the validator: {other:?}"
                    ),
                },
                Shape::RefusedMulti => match run_sync(&tool, &sql) {
                    Err(ScratchError::Refused(ScratchRejection::MultipleStatements {
                        count: 2,
                    })) => {}
                    other => prop_assert!(false, "multi-statement must be refused: {other:?}"),
                },
            }
            if shape == Shape::Write {
                for prepare in [
                    prepare_postgres_sql,
                    prepare_mysql_sql,
                    prepare_duckdb_sql,
                    prepare_snowflake_sql,
                    prepare_sqlite_sql,
                    prepare_clickhouse_sql,
                    prepare_bigquery_sql,
                ] {
                    prop_assert!(
                        prepare(&sql, 10).is_err(),
                        "the user-DB gate must still reject {sql}"
                    );
                }
            }
            run.sentinel_untouched(&sentinel);
        }
    }
}
