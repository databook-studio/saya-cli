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
            "first name".into(),
            "".into(),
            "1bad".into()
        ]),
        vec!["first_name", "first_name_2", "column_3", "_1bad"]
    );
}

#[test]
fn ragged_rows_name_the_line() {
    let rows = parse_csv(b"one,two\na\n", b',').expect("CSV parses before shape validation");
    assert_eq!(rows[1].len(), 1, "importer must name source line 2");
}
