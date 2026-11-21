//! The results `02` §3.3's preview and full view are read against, and the
//! readings their tests take of the rows they paint.

use ratatui::text::Line;
use saya_types::QueryResult;
use serde_json::{Value, json};

use super::transcript::TranscriptFacts;
use crate::agent::tools::CaptureRefusalReason;
use crate::front::tui::theme::{Charset, Scheme};
use crate::front::view::ResultPreview;
use crate::interactive::tui::wrap::cell_width;

/// The facts a preview paints with: the dark scheme and the UTF-8 glyph set.
pub(super) fn facts() -> TranscriptFacts {
    TranscriptFacts {
        scheme: Scheme::Dark,
        charset: Charset::Utf8,
        ..TranscriptFacts::default()
    }
}

/// One query's result, as the view model holds it.
pub(super) fn result(columns: &[&str], rows: Vec<Value>) -> ResultPreview {
    let row_count = rows.len();
    ResultPreview {
        connection: "demo".to_owned(),
        sql: "SELECT * FROM orders".to_owned(),
        result: QueryResult {
            columns: columns.iter().map(|column| (*column).to_owned()).collect(),
            rows,
            row_count,
            truncated: false,
            executed_sql: "SELECT * FROM orders".to_owned(),
        },
        refusal: None,
    }
}

/// A result the capture refused, its rows still in hand: none of them is
/// previewed.
pub(super) fn refused(reason: CaptureRefusalReason) -> ResultPreview {
    let mut preview = orders(6);
    preview.refusal = Some(reason);
    preview
}

/// `count` rows of one column: the preview's row cap.
pub(super) fn orders(count: usize) -> ResultPreview {
    let rows = (1..=count).map(|n| json!([n])).collect();
    result(&["order_id"], rows)
}

/// A table of `columns` numbered columns, one row: the columns a width has to
/// leave out.
pub(super) fn wide(columns: usize) -> ResultPreview {
    let names: Vec<String> = (1..=columns).map(|n| format!("column_{n:02}")).collect();
    let row: Vec<Value> = (1..=columns).map(|n| json!(n)).collect();
    let headers: Vec<&str> = names.iter().map(String::as_str).collect();
    result(&headers, vec![Value::Array(row)])
}

/// The rows as a terminal paints them, one string per row.
pub(super) fn rendered(rows: &[Line<'_>]) -> Vec<String> {
    rows.iter().map(words).collect()
}

/// One row as a terminal paints it.
pub(super) fn words(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

/// The cells one row occupies.
pub(super) fn cells(line: &Line<'_>) -> usize {
    cell_width(&words(line))
}
