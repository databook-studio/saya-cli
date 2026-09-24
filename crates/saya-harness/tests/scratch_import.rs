//! Behavioural coverage for the contained CSV-to-scratch import.

use std::sync::Arc;

use saya_agent::ToolExecutor;
use saya_harness::{
    scratch::{ScratchSql, parse_csv, sanitize_headers},
    workspace::Workspace,
};
use serde_json::json;

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
            json!({"sql":"SELECT * FROM read_csv('ok.csv')"})
        )
        .await
        .is_err()
    );
    let _ = (std::fs::remove_dir_all(root), std::fs::remove_file(outside));
}
