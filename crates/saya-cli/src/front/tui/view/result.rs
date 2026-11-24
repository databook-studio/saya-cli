//! The SQL line, the result's header and its preview table, under the block
//! the answer rests on (`02` §3.3).
//!
//! A preview shows the first columns that fit and at most five rows; the table
//! itself is [`result_table`]'s, which paints today's table rendering in the
//! transcript's tokens. A capture that was refused has no rows to preview, so
//! the block says why in the one dim line the spec fixes.

use ratatui::text::Line;

use super::result_table;
use super::transcript::TranscriptFacts;
use super::turn_rows::{GUTTER, plain};
use crate::agent::tools::CaptureRefusalReason;
use crate::front::tui::theme::Token;
use crate::front::view::{ResultPreview, sql_one_line};

/// The query, the result's header and the preview's rows.
pub(crate) fn rows(
    preview: &ResultPreview,
    facts: &TranscriptFacts,
    content: usize,
) -> Vec<Line<'static>> {
    let sql = sql_one_line(&preview.sql, content.saturating_sub(GUTTER));
    let mut out = vec![plain(GUTTER, &sql, Token::Code, facts, content)];
    match preview.refusal {
        // A refused capture has no rows to preview: the one line says why.
        Some(reason) => out.push(plain(GUTTER, refusal(reason), Token::Dim, facts, content)),
        None => {
            let head = format!(
                "result · {} · {}",
                preview.connection,
                rows_text(preview.result.row_count)
            );
            out.push(plain(GUTTER, &head, Token::Dim, facts, content));
            out.extend(result_table::preview(preview, facts, content));
        }
    }
    out
}

/// Why the preview holds no rows: `02` §3.3's one dim line, which never
/// pretends the result was empty. `/export` says the same line when it is
/// asked to write a capture that was refused.
pub(crate) fn refusal(reason: CaptureRefusalReason) -> &'static str {
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
