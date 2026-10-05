//! Behavioural coverage for the contained CSV-to-scratch import.

use std::{
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};

use saya_agent::{CancellationToken, MAX_TOOL_MESSAGE_BYTES, ToolExecutionContext, ToolExecutor};
use saya_harness::{
    scratch::{ScratchSql, parse_csv, sanitize_headers},
    workspace::Workspace,
};
use serde_json::json;

/// Every await sits under a 60 s guard so a hung import FAILS instead of
/// stalling the suite — the same discipline as the `scratch_sql` battery.
const GUARD: Duration = Duration::from_secs(60);

async fn guarded<F: Future>(future: F) -> F::Output {
    tokio::time::timeout(GUARD, future)
        .await
        .expect("the 60s guard: a hang means the cancel-before-commit contract regressed")
}

/// The loop-assigned context: the loop's tool-message cap and the turn's
/// token (the import applies its own row, byte and wall-clock budgets).
fn context(token: &CancellationToken) -> ToolExecutionContext {
    ToolExecutionContext {
        result_cap: MAX_TOOL_MESSAGE_BYTES,
        cancellation: token.clone(),
    }
}

#[test]
fn imports_a_quoted_csv_into_varchar_rows() {
    let rows = parse_csv(
        b"name,note\r\nAda,\"one, two\"\r\nGrace,\"said \"\"hi\"\"\nand left\"\r\n",
        b',',
    )
    .expect("quoted RFC 4180 input parses");
    assert_eq!(rows[0], vec!["name", "note"]);
    assert_eq!(rows[1], vec!["Ada", "one, two"]);
    assert_eq!(rows[2], vec!["Grace", "said \"hi\"\nand left"]);
}

#[tokio::test]
async fn imports_a_quoted_csv_into_a_varchar_table() {
    let root = std::env::temp_dir().join(format!("saya-scratch-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temporary workspace");
    std::fs::write(
        root.join("people.csv"),
        b"name,note\r\nAda,\"one, two\"\r\nGrace,\"said \"\"hi\"\"\nand left\"\r\n",
    )
    .expect("CSV fixture");
    let workspace = Arc::new(Workspace::open(&root).expect("bound workspace"));
    let tool = ScratchSql::open(&root)
        .expect("scratch opens")
        .with_workspace(workspace);
    let imported = tool
        .execute(
            "scratch_import",
            json!({"path": "people.csv", "table": "people"}),
        )
        .await
        .expect("import succeeds");
    assert_eq!(imported["columns"], json!(["name", "note"]));
    assert_eq!(imported["rows_imported"], 2);
    let selected = tool
        .execute(
            "scratch_sql",
            json!({"sql": "SELECT * FROM people ORDER BY name"}),
        )
        .await
        .expect("query succeeds");
    assert_eq!(
        selected["rows"],
        json!([["Ada", "one, two"], ["Grace", "said \"hi\"\nand left"]])
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn imports_headerless_csv_with_a_custom_delimiter() {
    let root = std::env::temp_dir().join(format!("saya-headerless-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("data.csv"), b"Ada;42\nGrace;99\n").unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    let result = tool
        .execute(
            "scratch_import",
            json!({"path":"data.csv","table":"people","header":false,"delimiter":";"}),
        )
        .await
        .unwrap();
    assert_eq!(result["columns"], json!(["column_1", "column_2"]));
    assert_eq!(result["rows_imported"], 2);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn invalid_utf8_is_refused() {
    assert!(parse_csv(b"name\n\xff\n", b',').is_err());
}

#[test]
fn malformed_quote_tails_and_unsafe_delimiters_are_refused() {
    for input in [b"a\"b\n".as_slice(), b"\"a\"junk\n".as_slice()] {
        assert!(
            parse_csv(input, b',').is_err(),
            "malformed quote syntax refuses"
        );
    }
    for delimiter in [b'"', b'\n', b'\r', 0] {
        assert!(
            parse_csv(b"a,b\n", delimiter).is_err(),
            "unsafe delimiter refuses"
        );
    }
    assert_eq!(
        parse_csv(b"\"\"", b',').expect("empty quoted field"),
        vec![vec!["".to_owned()]]
    );
}

#[test]
fn row_ceiling_refuses_more_than_five_hundred_thousand_data_rows() {
    let csv = b"\n".repeat(500_002);
    assert!(parse_csv(&csv, b',').is_err());
}

#[test]
fn header_names_are_sanitised_deterministically() {
    assert_eq!(
        sanitize_headers(&[
            "first name".into(),
            "FIRST NAME".into(),
            "first name".into(),
            "".into(),
            "1bad".into()
        ]),
        vec![
            "first_name",
            "FIRST_NAME_2",
            "first_name_3",
            "column_4",
            "_1bad"
        ]
    );
}

#[test]
fn ragged_rows_name_the_line() {
    let rows = parse_csv(b"one,two\na\n", b',').expect("CSV parses before shape validation");
    assert_eq!(rows[1].len(), 1, "importer must name source line 2");
}

#[tokio::test]
async fn ragged_import_names_the_physical_record_line() {
    let root = std::env::temp_dir().join(format!("saya-ragged-import-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ragged.csv"), b"a,b\n\"one\ntwo\",ok\nonly\n").unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    let error = tool
        .execute(
            "scratch_import",
            json!({"path":"ragged.csv","table":"target"}),
        )
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("row 4"),
        "error names first ragged record's physical line: {error}"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[tokio::test]
async fn path_escapes_invalid_utf8_and_existing_tables_are_refused_or_replaced() {
    let root = std::env::temp_dir().join(format!("saya-import-policy-{}", std::process::id()));
    let outside = root.with_extension("outside.csv");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("ok.csv"), b"name\nnew\n").unwrap();
    std::fs::write(root.join("bad.csv"), b"name\n\xff\n").unwrap();
    std::fs::write(
        root.join("field.csv"),
        format!("name\n{}\n", "x".repeat(64 * 1024 + 1)),
    )
    .unwrap();
    let wide = format!(
        "{}\n{}\n",
        (0..513)
            .map(|i| format!("c{i}"))
            .collect::<Vec<_>>()
            .join(","),
        (0..513).map(|_| "x").collect::<Vec<_>>().join(",")
    );
    std::fs::write(root.join("wide.csv"), wide).unwrap();
    let huge = std::fs::File::create(root.join("huge.csv")).unwrap();
    huge.set_len((32 * 1024 * 1024 + 1) as u64).unwrap();
    std::fs::write(&outside, b"name\noutside\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, root.join("link.csv")).unwrap();
    let tool = ScratchSql::open(&root)
        .unwrap()
        .with_workspace(Arc::new(Workspace::open(&root).unwrap()));
    tool.execute(
        "scratch_sql",
        json!({"sql":"CREATE TABLE target (name VARCHAR)"}),
    )
    .await
    .unwrap();
    tool.execute(
        "scratch_sql",
        json!({"sql":"INSERT INTO target VALUES ('old')"}),
    )
    .await
    .unwrap();
    for arguments in [
        json!({"path":"ok.csv","table":"target","delimiter":1}),
        json!({"path":"ok.csv","table":"target","if_exists":null}),
    ] {
        assert!(tool.execute("scratch_import", arguments).await.is_err());
    }
    assert!(
        tool.execute(
            "scratch_import",
            json!({"path":"huge.csv","table":"target","if_exists":"replace"})
        )
        .await
        .is_err()
    );
    let unchanged = tool
        .execute("scratch_sql", json!({"sql":"SELECT name FROM target"}))
        .await
        .unwrap();
    assert_eq!(unchanged["rows"], json!([["old"]]));
    for path in ["../escape.csv", outside.to_str().unwrap(), "bad.csv"] {
        assert!(
            tool.execute(
                "scratch_import",
                json!({"path":path,"table":"target","if_exists":"replace"})
            )
            .await
            .is_err()
        );
    }
    #[cfg(unix)]
    assert!(
        tool.execute(
            "scratch_import",
            json!({"path":"link.csv","table":"target","if_exists":"replace"})
        )
        .await
        .is_err()
    );
    for path in ["wide.csv", "field.csv"] {
        assert!(
            tool.execute(
                "scratch_import",
                json!({"path":path,"table":"target","if_exists":"replace"})
            )
            .await
            .is_err()
        );
        let unchanged = tool
            .execute("scratch_sql", json!({"sql":"SELECT name FROM target"}))
            .await
            .unwrap();
        assert_eq!(
            unchanged["rows"],
            json!([["old"]]),
            "{path} must preserve target"
        );
    }
    assert!(
        tool.execute("scratch_import", json!({"path":"ok.csv","table":"target"}))
            .await
            .is_err()
    );
    tool.execute(
        "scratch_import",
        json!({"path":"ok.csv","table":"target","if_exists":"replace"}),
    )
    .await
    .unwrap();
    let rows = tool
        .execute("scratch_sql", json!({"sql":"SELECT name FROM target"}))
        .await
        .unwrap();
    assert_eq!(rows["rows"], json!([["new"]]));
    assert!(
        tool.execute(
            "scratch_sql",
            json!({"sql":format!("SELECT * FROM read_csv('{}')", root.join("ok.csv").display())})
        )
        .await
        .is_err()
    );
    let _ = (std::fs::remove_dir_all(root), std::fs::remove_file(outside));
}

// ---------------------------------------------------------------------------
// Turn cancellation: a cancelled turn stops a scratch import cooperatively —
// checked before parsing, beside the lock, every row, and immediately before
// the commit; a cancelled import never commits, and the blocking task is
// joined (the await is the settle) before the call returns.

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
