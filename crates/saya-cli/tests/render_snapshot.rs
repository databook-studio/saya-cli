//! Exemplar snapshot test (see docs/standards/testing.md).
//!
//! `render_event` turns a `TerminalEvent` into terminal / JSON output. The exact
//! shape of that output is tedious to assert by hand and easy to regress, so we pin
//! it with `insta`. Review every snapshot change like code: `cargo insta review`.

use saya_cli::{RenderFormat, TerminalEvent, render_event};
use saya_types::QueryResult;
use serde_json::json;

fn sample_query_result() -> QueryResult {
    QueryResult {
        columns: vec!["id".into(), "name".into()],
        rows: vec![json!([1, "alpha"]), json!([2, "beta"])],
        row_count: 2,
        truncated: true,
        executed_sql: "SELECT id, name FROM t LIMIT 2".into(),
    }
}

#[test]
fn text_render_of_query_result_is_stable() {
    let event = TerminalEvent::QueryResult {
        result: sample_query_result(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Text).stdout);
}

#[test]
fn json_render_of_query_result_is_stable() {
    let event = TerminalEvent::QueryResult {
        result: sample_query_result(),
    };
    insta::assert_snapshot!(render_event(&event, RenderFormat::Json).stdout);
}
