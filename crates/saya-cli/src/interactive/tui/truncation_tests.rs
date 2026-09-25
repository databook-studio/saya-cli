//! Phase 6 packet 1 red tests: every surface that renders a capped result
//! says it is capped, in the existing words, and none claims a total.

use super::sql_task::{Followup, SqlTask, complete};
use super::table::{clip_table_block, format_markdown_tables, format_plan, format_table};
use super::transcript::{BlockKind, Transcript};
use super::types::WideTableView;

fn query_result(rows: usize, truncated: bool) -> saya_types::QueryResult {
    saya_types::QueryResult {
        columns: vec!["id".to_string()],
        rows: (0..rows).map(|i| serde_json::json!([i])).collect(),
        row_count: rows,
        truncated,
        executed_sql: "SELECT id FROM t".to_string(),
    }
}

/// Drives the chart follow-up with an explicit on-disk path (so no
/// temp-chart reservation runs) and returns the transcript's system note.
/// Writes a stub HTML file first so `write_html` overwrites it and the note
/// takes the "open it manually" branch (no browser is launched in tests) —
/// the truncation assertion only reads the suffix either way.
fn chart_note_for(result: &saya_types::QueryResult) -> String {
    let dir = std::env::temp_dir().join(format!("saya-p6p1-chart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("chart harness dir");
    let path = dir.join(format!(
        "chart-{}.html",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    std::fs::write(&path, "<html></html>").expect("stub chart file");
    let task = SqlTask {
        profile: None,
        sql: "SELECT id FROM t".to_string(),
        followup: Followup::Chart {
            kind: None,
            path: Some(path.to_string_lossy().into_owned()),
        },
    };
    // One numeric column charts as a bar; `suggest_spec` picks it up.
    let chartable = saya_types::QueryResult {
        columns: vec!["id".to_string()],
        rows: result.rows.clone(),
        row_count: result.row_count,
        truncated: result.truncated,
        executed_sql: result.executed_sql.clone(),
    };
    let mut transcript = Transcript::new();
    complete(
        &task,
        crate::render::TerminalEvent::QueryResult { result: chartable },
        &mut transcript,
        &mut None,
    );
    let _ = std::fs::remove_file(&path);
    transcript
        .blocks()
        .iter()
        .rev()
        .find(|b| b.kind == BlockKind::System)
        .map(|b| b.text.clone())
        .unwrap_or_default()
}

#[test]
fn a_chart_from_a_capped_result_says_so() {
    let note = chart_note_for(&query_result(3, true));
    assert!(
        note.contains("(result was truncated)"),
        "capped chart note must say so: {note:?}"
    );
}

#[test]
fn a_chart_from_a_complete_result_says_nothing_extra() {
    let note = chart_note_for(&query_result(3, false));
    assert!(
        !note.contains("truncat"),
        "complete chart note must not cry wolf: {note:?}"
    );
}

#[test]
fn a_markdown_table_names_its_row_count() {
    let input = "| id |\n|---|\n| 1 |\n| 2 |";
    let output = format_markdown_tables(input);
    assert!(
        output.contains("2 row(s)"),
        "markdown table must name its row count: {output:?}"
    );
}

#[test]
fn a_capped_markdown_table_says_it_is_capped() {
    let input = "| id |\n|---|\n| 1 |\n| 2 |\n\n<!-- truncated -->";
    let output = format_markdown_tables(input);
    assert!(
        output.contains("2 row(s) (truncated)"),
        "capped markdown table must say so: {output:?}"
    );
}

#[test]
fn a_capped_plan_says_it_is_capped() {
    let output = format_plan(&query_result(2, true));
    assert!(
        output.contains("(truncated)"),
        "capped plan must say so: {output:?}"
    );
}

#[test]
fn no_surface_claims_a_total() {
    let capped = query_result(2, true);
    let surfaces = vec![
        chart_note_for(&capped),
        format_markdown_tables("| id |\n|---|\n| 1 |\n| 2 |\n\n<!-- truncated -->"),
        format_plan(&capped),
    ];
    for text in &surfaces {
        for banned in [" of ", "total", "all rows"] {
            assert!(
                !text.to_lowercase().contains(banned),
                "surface must not invent a denominator ({banned:?}): {text:?}"
            );
        }
    }
}

#[test]
fn a_footer_does_not_break_the_one_to_one_clip() {
    let input = "| id | name |\n|---|---|\n| 1 | alice |\n| 2 | bob |";
    let rendered = format_markdown_tables(input);
    let lines: Vec<String> = rendered.lines().map(str::to_string).collect();
    let wv = WideTableView {
        h_offset: 0,
        pin_first: false,
        columns: None,
    };
    let out = clip_table_block(&lines, &wv, 30);
    assert_eq!(out.len(), lines.len(), "clipping never adds or drops lines");
    assert!(
        out.iter().any(|l| l.contains("2 row(s)")),
        "footer must survive clipping: {out:?}"
    );
    let _ = format_table(&query_result(2, false));
}
