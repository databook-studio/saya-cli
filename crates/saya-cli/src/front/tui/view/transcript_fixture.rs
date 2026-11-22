//! The database turn of `02` §2 as the view model holds it, and the readings
//! the transcript's tests take of the rows it paints.

use std::time::Duration;

use ratatui::text::Line;
use saya_types::QueryResult;
use serde_json::{Value, json};

use super::transcript::TranscriptFacts;
use crate::front::tui::theme::{Charset, Scheme};
use crate::front::view::{
    Activity, Answer, Entry, ResultPreview, Step, StepState, TurnView, UserMessage,
};
use crate::interactive::tui::wrap::cell_width;

/// The prompt `02` §2 opens with.
pub(super) const PROMPT: &str = "how many orders did we get since December?";

/// The SQL the answer rests on (`02` §3.3).
pub(super) const SQL: &str = "SELECT COUNT(*) AS orders_since_december FROM orders";

/// The answer body `02` §2 draws, as the model writes it: a paragraph, a
/// lead-in line and two bullets.
pub(super) const ANSWER: &str = concat!(
    "110 orders since December 2025 (2025-12-01 through 2026-01-01, the latest date in the data).\n",
    "\n",
    "Notes on interpretation:\n",
    "- The data contains only one December (2025), so \"since December\" = from 2025-12-01 ",
    "onward, which includes the 5 orders on 2026-01-01. December 2025 alone would be 105.\n",
    "- I counted all orders regardless of status (completed, pending, refunded) — 269 ",
    "completed, 144 pending, 147 refunded overall. Say the word if you want only completed."
);

/// The demo turn's time (`02` §2: `14s`).
pub(super) const ELAPSED: Duration = Duration::from_secs(14);

/// The facts the pane paints with: one turn's time, a workspace root, the dark
/// scheme and the UTF-8 glyph set.
pub(super) fn facts(elapsed: Duration) -> TranscriptFacts {
    TranscriptFacts {
        elapsed: vec![elapsed],
        workspace: Some("~/shop".to_owned()),
        scheme: Scheme::Dark,
        charset: Charset::Utf8,
        ..TranscriptFacts::default()
    }
}

/// A step that settled with `summary` as its outcome.
pub(super) fn done(tool: &str, arguments: Value, summary: &str) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state: StepState::Done {
            summary: summary.to_owned(),
        },
    }
}

/// A step the call asked for and no outcome reached yet.
pub(super) fn running(tool: &str, arguments: Value) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state: StepState::Running,
    }
}

/// One query call's arguments: the connection it named and the SQL it ran.
pub(super) fn query(sql: &str) -> Value {
    json!({"connection": "demo", "sql": sql})
}

/// The four settled steps of `02` §2: a schema read and three queries.
fn demo_steps() -> Vec<Step> {
    vec![
        done(
            "schema_discovery",
            json!({"connection": "demo"}),
            "discovered 12 tables in demo",
        ),
        done(
            "bounded_sql_query",
            query("SELECT status, COUNT(*) AS n FROM orders GROUP BY status"),
            "3 rows",
        ),
        done(
            "bounded_sql_query",
            query("SELECT order_date, COUNT(*) AS n FROM orders"),
            "35 rows",
        ),
        done("bounded_sql_query", query(SQL), "1 row"),
    ]
}

/// The result one successful query handed the front.
fn preview(refusal: Option<crate::agent::tools::CaptureRefusalReason>) -> ResultPreview {
    ResultPreview {
        connection: "demo".to_owned(),
        sql: SQL.to_owned(),
        result: QueryResult {
            columns: vec!["orders_since_december".to_owned()],
            rows: vec![json!([110])],
            row_count: 1,
            truncated: false,
            executed_sql: SQL.to_owned(),
        },
        refusal,
        evidence: None,
    }
}

/// The turn of `02` §2: the operator's message, four settled steps, the answer
/// and the result the answer rests on. `folded` is the activity block's own
/// state, which the reducer sets when the turn ends.
pub(super) fn database_turn(folded: bool) -> TurnView {
    let mut turn = TurnView::default();
    turn.push(Entry::UserMessage(UserMessage {
        text: PROMPT.to_owned(),
    }));
    turn.push(Entry::Activity(Activity {
        steps: demo_steps(),
        folded,
        failed: false,
    }));
    turn.push(Entry::Answer(Answer {
        text: ANSWER.to_owned(),
    }));
    turn.push(Entry::ResultPreview(preview(None)));
    turn
}

/// The same turn while it runs: six steps, the block live.
pub(super) fn live_turn() -> TurnView {
    let mut steps = demo_steps();
    steps.push(done("bounded_sql_query", query("SELECT 1"), "2 rows"));
    steps.push(done("bounded_sql_query", query("SELECT 2"), "5 rows"));
    steps.push(running("bounded_sql_query", query("SELECT 3")));
    let mut turn = TurnView::default();
    turn.push(Entry::UserMessage(UserMessage {
        text: PROMPT.to_owned(),
    }));
    turn.push(Entry::Activity(Activity {
        steps,
        folded: false,
        failed: false,
    }));
    turn
}

/// A block the view model has folded although a step failed — the state the
/// pane's own guard exists for.
pub(super) fn failed_turn() -> TurnView {
    let steps = vec![
        done(
            "schema_discovery",
            json!({"connection": "demo"}),
            "discovered 12 tables in demo",
        ),
        Step {
            tool: "bounded_sql_query".to_owned(),
            arguments: query(SQL),
            state: StepState::Failed {
                summary: "failed to complete: connection refused".to_owned(),
            },
        },
    ];
    let mut turn = TurnView::default();
    turn.push(Entry::UserMessage(UserMessage {
        text: PROMPT.to_owned(),
    }));
    turn.push(Entry::Activity(Activity {
        steps,
        folded: true,
        failed: true,
    }));
    turn
}

/// The answer block alone, `count` short lines of it: the live edge's fixture.
pub(super) fn answer_turn(count: usize) -> TurnView {
    let text: Vec<String> = (1..=count).map(|line| format!("line {line:02}")).collect();
    let mut turn = TurnView::default();
    turn.push(Entry::Answer(Answer {
        text: text.join("\n"),
    }));
    turn
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
