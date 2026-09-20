//! Phase 6 packet 2 red tests: a rendered result names the connection it came
//! from and the query that produced it, and keeps naming them after the
//! active connection changes.
//!
//! The scope line is baked into the table block text at completion time
//! (`sql_task::complete`, from the `Followup::Sql { connection }` value
//! captured at dispatch) — the same freezing packet 1's footer relies on — so
//! copy reads it through `block.text` like every other painted line.

use super::sql_task::{Followup, SqlTask, complete};
use super::table::{clip_table_block, format_table};
use super::transcript::{Block, BlockKind, Transcript};
use super::types::WideTableView;

fn query_result(sql: &str, rows: usize, truncated: bool) -> saya_types::QueryResult {
    saya_types::QueryResult {
        columns: vec!["region".to_string()],
        rows: (0..rows).map(|i| serde_json::json!([i])).collect(),
        row_count: rows,
        truncated,
        executed_sql: sql.to_string(),
    }
}

fn sql_task_with_connection(connection: Option<&str>, sql: &str) -> SqlTask {
    SqlTask {
        profile: connection.map(str::to_string),
        sql: sql.to_string(),
        followup: Followup::Sql {
            connection: connection.map(str::to_string),
        },
    }
}

/// Drives `complete` with a `QueryResult` event and returns the table block
/// it pushed: the only path that renders a direct-SQL result.
fn completed_table(task: &SqlTask, result: saya_types::QueryResult) -> Block {
    let mut transcript = Transcript::new();
    complete(
        task,
        crate::render::TerminalEvent::QueryResult { result },
        &mut transcript,
        &mut None,
    );
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == BlockKind::Table)
        .expect("complete pushes a table block")
        .clone()
}

#[test]
fn a_result_names_the_connection_it_came_from() {
    let task = sql_task_with_connection(Some("sales-demo"), "select region from orders");
    let block = completed_table(&task, query_result("select region from orders", 3, false));
    assert!(
        block.text.contains("sales-demo"),
        "scope line must name the captured connection: {:?}",
        block.text
    );
}

#[test]
fn a_result_names_the_query_that_produced_it() {
    let task = sql_task_with_connection(Some("sales-demo"), "select region, count(*) from orders");
    let block = completed_table(
        &task,
        query_result("select region, count(*) from orders", 3, false),
    );
    assert!(
        block.text.contains("select region, count(*) from orders"),
        "scope line must name the query: {:?}",
        block.text
    );
}

#[test]
fn a_result_keeps_its_connection_after_the_active_one_changes() {
    let task = sql_task_with_connection(Some("sales-demo"), "select region from orders");
    let block = completed_table(&task, query_result("select region from orders", 3, false));
    drop(task);
    // `state.profile` moves on; the block text was frozen at completion.
    let _new_active = Some("billing".to_string());
    assert!(
        block.text.contains("sales-demo"),
        "historical result keeps its original connection: {:?}",
        block.text
    );
    assert!(
        !block.text.contains("billing"),
        "a later connection must not relabel it: {:?}",
        block.text
    );
}

#[test]
fn a_result_with_no_connection_says_nothing_about_one() {
    let task = sql_task_with_connection(None, "select region from orders");
    let block = completed_table(&task, query_result("select region from orders", 3, false));
    let scope = block.text.lines().last().unwrap_or_default().to_string();
    assert!(
        !scope.contains("from ") || scope.contains("select region from orders"),
        "with no connection the line carries only the query: {scope:?}"
    );
    assert!(
        !scope.starts_with("from "),
        "no connection means no `from <profile>` prefix: {scope:?}"
    );
}

#[test]
fn the_scope_line_claims_no_total() {
    let task = sql_task_with_connection(Some("sales-demo"), "select region from orders");
    let block = completed_table(&task, query_result("select region from orders", 3, true));
    for banned in [" of ", "total", "all rows"] {
        assert!(
            !block.text.to_lowercase().contains(banned),
            "scope line must not invent a denominator ({banned:?}): {:?}",
            block.text
        );
    }
}

#[test]
fn the_scope_line_does_not_break_the_one_to_one_clip() {
    let task = sql_task_with_connection(Some("sales-demo"), "select region from orders");
    let block = completed_table(&task, query_result("select region from orders", 3, false));
    let lines: Vec<String> = block.text.lines().map(str::to_string).collect();
    let wv = WideTableView {
        h_offset: 0,
        pin_first: false,
        columns: None,
    };
    let out = clip_table_block(&lines, &wv, 30);
    assert_eq!(out.len(), lines.len(), "clipping never adds or drops lines");
    assert!(
        out.iter().any(|l| l.contains("sales-demo")),
        "scope line must survive clipping: {out:?}"
    );
    // A plain table with no scope still clips 1:1.
    let plain: Vec<String> = format_table(&query_result("select 1", 1, false))
        .lines()
        .map(str::to_string)
        .collect();
    let plain_out = clip_table_block(&plain, &wv, 30);
    assert_eq!(plain_out.len(), plain.len());
}

#[test]
fn the_scope_line_does_not_change_what_copy_yields() {
    let task = sql_task_with_connection(Some("sales-demo"), "select region from orders");
    let block = completed_table(&task, query_result("select region from orders", 3, false));
    // Copy reads `block.text` verbatim: the scope line rides along like the
    // packet-1 footer, never filtered, never rewritten.
    assert_eq!(block.text, block.text.clone());
    assert!(
        block.text.contains("3 row(s)"),
        "footer still present: {:?}",
        block.text
    );
    assert!(
        block.text.contains("sales-demo"),
        "scope line copied with the block: {:?}",
        block.text
    );
}
