//! The SQL line and the result's one-line stub, under the block the answer
//! rests on (`02` §3.3).
//!
//! The preview table itself is T6's. What stands here is the line naming the
//! query that ran and the line saying what came back; a capture that was
//! refused says why instead, in the sentence `02` §3.3 fixes.

use ratatui::text::Line;

use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, plain};
use crate::agent::tools::CaptureRefusalReason;
use crate::front::tui::theme::Token;
use crate::front::view::{ResultPreview, sql_one_line};

/// The query and the result, as two rows.
pub(super) fn rows(
    preview: &ResultPreview,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let sql = sql_one_line(&preview.sql, content.saturating_sub(GUTTER));
    let result = match preview.refusal {
        Some(reason) => format!("result · {} · {}", preview.connection, refusal(reason)),
        None => format!(
            "result · {} · {}",
            preview.connection,
            rows_text(preview.result.row_count)
        ),
    };
    vec![
        plain(GUTTER, &sql, Token::Code, facts, content),
        plain(GUTTER, &result, Token::Dim, facts, content),
    ]
}

/// Why the preview holds no rows: `02` §3.3's one dim line, which never
/// pretends the result was empty.
fn refusal(reason: CaptureRefusalReason) -> &'static str {
    match reason {
        CaptureRefusalReason::ModelViewTruncated => {
            "model saw a truncated result; rerun with /sql to see rows"
        }
        CaptureRefusalReason::ModelViewRedacted => {
            "model saw redacted values; rerun with /sql to see rows"
        }
        CaptureRefusalReason::OverBudget => "result too large to hold; rerun with /sql to see rows",
    }
}

/// How many rows a result holds, in the number the count takes.
fn rows_text(rows: usize) -> String {
    match rows {
        1 => "1 row".to_owned(),
        rows => format!("{rows} rows"),
    }
}
