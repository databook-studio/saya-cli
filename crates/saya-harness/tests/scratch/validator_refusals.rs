use super::*;

#[tokio::test]
async fn a_file_reading_function_is_refused_by_the_validator_by_name_before_duckdb() {
    let run = TempRun::new("file-function");
    let tool = admitted(&run);
    for (sql, name) in [
        ("SELECT * FROM read_csv('x.csv')", "read_csv"),
        ("SELECT * FROM read_csv_auto('x.csv')", "read_csv_auto"),
        ("SELECT * FROM read_parquet('x.parquet')", "read_parquet"),
        ("SELECT * FROM read_json_auto('x.json')", "read_json_auto"),
        ("SELECT read_text('x.txt')", "read_text"),
        ("SELECT * FROM glob('*.csv')", "glob"),
        ("SELECT * FROM sqlite_scan('x.db', 't')", "sqlite_scan"),
        ("SELECT * FROM parquet_scan('x.parquet')", "parquet_scan"),
        ("SELECT http_get('http://127.0.0.1:1/x')", "http_get"),
        // Hidden behind a schema qualification and behind a write:
        ("SELECT * FROM main.read_csv('x.csv')", "read_csv"),
        ("INSERT INTO t SELECT * FROM read_csv('x.csv')", "read_csv"),
        (
            "CREATE TABLE c AS SELECT * FROM read_parquet('x.parquet')",
            "read_parquet",
        ),
    ] {
        match tool.run(sql).await {
            Err(ScratchError::Refused(ScratchRejection::FileFunction(refused))) => {
                assert_eq!(
                    refused, name,
                    "the refusal must name the file reader: {sql}"
                );
            }
            other => panic!("{sql} must be refused by the validator, got {other:?}"),
        }
    }
    // The refusals all happened before any statement reached DuckDB: nothing
    // was executed. Reopen only after the tool is dropped — the file lock
    // belongs to the tool's connection.
    drop(tool);
    let conn = reopened_scratch(&run);
    let tables = conn
        .query_row(
            "SELECT count(*) FROM information_schema.tables WHERE table_schema NOT IN ('information_schema','pg_catalog')",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    assert_eq!(
        tables, 0,
        "refused statements must not have created anything"
    );
}

#[tokio::test]
async fn a_file_reading_function_hidden_in_a_write_is_refused_before_duckdb() {
    let run = TempRun::new("hidden-in-write");
    let tool = admitted(&run);
    tool.run("CREATE TABLE stage(a INTEGER)").await.unwrap();
    match tool
        .run("INSERT INTO stage SELECT * FROM read_csv('x.csv')")
        .await
    {
        Err(ScratchError::Refused(ScratchRejection::FileFunction(name))) => {
            assert_eq!(name, "read_csv");
        }
        other => panic!("must be refused by the validator, got {other:?}"),
    }
    // The refused statement never reached the engine: the staging table is
    // still empty. Reopen only after the tool is dropped — the file lock
    // belongs to the tool's connection.
    drop(tool);
    let conn = reopened_scratch(&run);
    let rows: i64 = conn
        .query_row("SELECT count(*) FROM stage", [], |row| row.get(0))
        .unwrap();
    assert_eq!(rows, 0);
}

/// The backstop, asserted separately: DuckDB's own permission layer refuses
/// the same statements — install, load, an http read *and a local file read* —
/// with external access off. The flag is all-or-nothing; this is the second,
/// independent half of the belt-and-braces.
#[test]
fn duckdb_itself_refuses_file_reads_at_the_permission_layer() {
    let run = TempRun::new("backstop");
    let sentinel = run.plant_sentinel();
    let config = duckdb::Config::default()
        .access_mode(duckdb::AccessMode::ReadWrite)
        .and_then(|item| item.enable_external_access(false))
        .and_then(|item| item.enable_autoload_extension(false))
        .and_then(|item| item.with("allow_community_extensions", "false"))
        .and_then(|item| item.with("allow_persistent_secrets", "false"))
        .and_then(|item| item.with("lock_configuration", "true"))
        .expect("the pinned scratch configuration must build");
    let conn = duckdb::Connection::open_with_flags(run.scratch_file(), config).unwrap();
    for sql in [
        "INSTALL httpfs".to_string(),
        "LOAD httpfs".to_string(),
        format!("SELECT * FROM read_csv('{}')", sentinel.display()),
        format!("SELECT * FROM '{}'", sentinel.display()),
    ] {
        let error = conn
            .execute_batch(&sql)
            .expect_err("every file route must be refused")
            .to_string();
        assert!(
            error.contains("Permission Error"),
            "{sql} was not refused at the permission layer: {error}"
        );
    }
    run.sentinel_untouched(&sentinel);
}

#[tokio::test]
async fn install_and_load_are_refused_by_the_validator() {
    let run = TempRun::new("install-load");
    let tool = admitted(&run);
    for sql in ["INSTALL httpfs", "LOAD httpfs", "INSTALL json", "LOAD icu"] {
        match tool.run(sql).await {
            Err(ScratchError::Refused(ScratchRejection::Statement(kind))) => {
                assert!(
                    kind.starts_with(sql.split(' ').next().unwrap()),
                    "{sql}: {kind}"
                );
            }
            other => panic!("{sql} must be refused by the validator, got {other:?}"),
        }
    }
    run.sentinel_untouched(&run.plant_sentinel());
}

#[tokio::test]
async fn multi_statement_input_is_refused() {
    let run = TempRun::new("multi-statement");
    let tool = admitted(&run);
    for sql in [
        "SELECT 1; SELECT 2",
        "CREATE TABLE t(a INTEGER); INSERT INTO t VALUES (1)",
        "SELECT 1; INSTALL httpfs",
    ] {
        match tool.run(sql).await {
            Err(ScratchError::Refused(ScratchRejection::MultipleStatements { count })) => {
                assert_eq!(count, 2, "{sql}");
            }
            other => panic!("{sql} must be refused, got {other:?}"),
        }
    }
}
