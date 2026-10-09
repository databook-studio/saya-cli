use super::*;

/// A token already cancelled refuses the import before the workspace file is
/// even read: the error text says "cancelled", the target table was never
/// created, and a normal import afterwards still succeeds.
#[tokio::test]
async fn a_precancelled_import_touches_nothing() {
    let root = std::env::temp_dir().join(format!("saya-import-precancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ok.csv"), b"name\nnew\n").unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    let token = CancellationToken::new();
    token.cancel();
    let error = guarded(tool.execute_with_context(
        "scratch_import",
        json!({"path": "ok.csv", "table": "t"}),
        context(&token),
    ))
    .await
    .expect_err("a pre-cancelled import is refused, never completed");
    assert!(error.to_string().contains("cancelled"), "{error}");
    // The target table does not exist afterwards: a catalog error, not a count.
    let missing = guarded(tool.execute("scratch_sql", json!({"sql": "SELECT count(*) FROM t"})))
        .await
        .expect_err("the pre-cancelled import must have had no effect");
    assert!(
        missing.to_string().contains("Catalog Error"),
        "the table must be absent: {missing}"
    );
    // The connection is unharmed: a normal import then succeeds.
    let imported = guarded(tool.execute("scratch_import", json!({"path": "ok.csv", "table": "t"})))
        .await
        .unwrap();
    assert_eq!(imported["columns"], json!(["name"]));
    assert_eq!(imported["rows_imported"], 1);
    let _ = std::fs::remove_dir_all(root);
}

/// A cancel arriving mid-import is honoured before the transaction commits:
/// in `replace` mode the original table survives untouched, no staging table
/// remains, the blocking task is joined before the call returns, and the
/// connection still serves. The replacement CSV is sized so an uncancelled
/// import cannot finish anywhere near the cancel delay nor slip past the
/// 60 s IMPORT_TIMEOUT (measured on the dev machine, debug build: ~412 ms to
/// parse the 9.7 MB file, ~476 µs per inserted row — ~214 s for 450k rows).
/// The token is cancelled 2 s in — safely past the up-front parse, well
/// inside the row loop — because a cancel landing during the parse is only
/// observed at the next check, which would mask a lost per-row check.
#[tokio::test]
async fn cancelling_a_replacement_import_keeps_the_original_table_and_frees_the_connection() {
    let root = std::env::temp_dir().join(format!("saya-import-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let mut csv = String::from("c1,c2,c3,c4\n");
    for i in 0..450_000usize {
        let _ = std::fmt::Write::write_fmt(&mut csv, format_args!("{i},abc,{i},xyz\n"));
    }
    std::fs::write(root.join("big.csv"), csv).unwrap();
    std::fs::write(root.join("small.csv"), b"name\nold\n").unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    tool.execute(
        "scratch_import",
        json!({"path": "small.csv", "table": "target"}),
    )
    .await
    .unwrap();
    let token = CancellationToken::new();
    let call_token = token.clone();
    let importer = tool.clone();
    let started = Instant::now();
    let import_task = tokio::spawn(async move {
        guarded(importer.execute_with_context(
            "scratch_import",
            json!({"path": "big.csv", "table": "target", "if_exists": "replace"}),
            context(&call_token),
        ))
        .await
    });
    tokio::time::sleep(Duration::from_secs(2)).await;
    token.cancel();
    let error = guarded(import_task)
        .await
        .expect("the import task must not panic")
        .expect_err("a cancelled import is an error, never a result");
    let elapsed = started.elapsed();
    assert!(error.to_string().contains("cancelled"), "{error}");
    assert!(!error.to_string().contains("timed out"), "{error}");
    assert!(
        elapsed < Duration::from_secs(15),
        "cancellation must surface promptly, took {elapsed:?}"
    );
    // The ORIGINAL table survives; the replacement never committed.
    let rows = guarded(tool.execute("scratch_sql", json!({"sql": "SELECT name FROM target"})))
        .await
        .unwrap();
    assert_eq!(rows["rows"], json!([["old"]]));
    // No staging table remains from the rolled-back transaction.
    let staging = guarded(tool.execute(
        "scratch_sql",
        json!({"sql": "SELECT count(*) FROM duckdb_tables() WHERE table_name LIKE '__saya_import_%'"}),
    ))
    .await
    .unwrap();
    assert_eq!(staging["rows"], json!([[0]]));
    // The joined blocking task released the connection: it still serves.
    let answer = guarded(tool.execute("scratch_sql", json!({"sql": "SELECT 42"})))
        .await
        .unwrap();
    assert_eq!(answer["rows"], json!([[42]]));
    let _ = std::fs::remove_dir_all(root);
}

/// Regression: with an uncancelled context token the context path commits and
/// reports exactly what the plain path reports — row count and columns.
#[tokio::test]
async fn an_uncancelled_import_still_commits_and_reports_rows() {
    let root = std::env::temp_dir().join(format!("saya-import-uncancelled-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("people.csv"),
        b"name,note\r\nAda,\"one, two\"\r\nGrace,\"said \"\"hi\"\"\nand left\"\r\n",
    )
    .unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    let token = CancellationToken::new();
    let imported = guarded(tool.execute_with_context(
        "scratch_import",
        json!({"path": "people.csv", "table": "people"}),
        context(&token),
    ))
    .await
    .unwrap();
    assert_eq!(imported["table"], "people");
    assert_eq!(imported["columns"], json!(["name", "note"]));
    assert_eq!(imported["rows_imported"], 2);
    let selected = guarded(tool.execute_with_context(
        "scratch_sql",
        json!({"sql": "SELECT * FROM people ORDER BY name"}),
        context(&token),
    ))
    .await
    .unwrap();
    assert_eq!(
        selected["rows"],
        json!([["Ada", "one, two"], ["Grace", "said \"hi\"\nand left"]])
    );
    let _ = std::fs::remove_dir_all(root);
}
