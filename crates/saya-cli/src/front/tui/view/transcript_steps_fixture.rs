//! The fixtures the step-line tests are built from: a turn of two workspace
//! steps whose commands are long enough to wrap, a live turn whose last call is
//! still out, and a long turn for the compact cap.
//!
//! Kept apart from the tests so each file stays one concern: these are the
//! turns, and the assertions are about the rows they paint.

use std::time::Duration;

use serde_json::{Value, json};

use super::transcript::TranscriptFacts;
use super::transcript_fixture::rendered;
use super::transcript_rows;
use crate::front::view::{Activity, Answer, Entry, Step, StepState, TurnView, UserMessage};

/// The command the workspace turn ran: long enough that 60 columns wrap it.
pub(super) const COMMAND: &str =
    "python3 -m unittest tests/test_pricing.py tests/test_orders.py tests/test_refunds.py -v";
/// The file the turn read after its command.
pub(super) const PATH: &str = "pricing.py";

/// The demo turn's time.
pub(super) const ELAPSED: Duration = Duration::from_secs(14);

/// A step that settled with `summary` as the outcome it reported.
pub(super) fn done(tool: &str, arguments: Value, summary: &str) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state: StepState::Done {
            summary: summary.to_owned(),
        },
    }
}

/// A step whose call has no outcome yet.
pub(super) fn running(tool: &str, arguments: Value) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state: StepState::Running,
    }
}

/// A turn of `steps`, settled exactly as the reducer leaves one.
pub(super) fn turn(steps: Vec<Step>) -> TurnView {
    build(steps, true)
}

/// The same turn with its call still out: the block the reducer holds while a
/// step runs.
pub(super) fn live_turn(steps: Vec<Step>) -> TurnView {
    build(steps, false)
}

/// A turn of `steps`, as the block stands while the turn does.
pub(super) fn build(steps: Vec<Step>, ended: bool) -> TurnView {
    let mut turn = TurnView::default();
    turn.push(Entry::UserMessage(UserMessage {
        text: "check the pricing tests".to_owned(),
    }));
    turn.push(Entry::Activity(Activity {
        steps,
        ended,
        folded: false,
        failed: false,
    }));
    turn.push(Entry::Answer(Answer {
        text: "The pricing tests fail on the old total.".to_owned(),
    }));
    turn
}

/// The turn this job is about: two steps — a command and a read — with the
/// outcomes they reported.
pub(super) fn two_step_turn() -> TurnView {
    turn(vec![
        done("shell", json!({"command": COMMAND}), "python3 exited 1"),
        done("read", json!({"path": PATH}), "read 40 lines"),
    ])
}

/// A turn of `count` settled steps, for the compact cap.
pub(super) fn many_step_turn(count: usize) -> TurnView {
    let steps = (1..=count)
        .map(|index| {
            done(
                "read",
                json!({"path": format!("file{index}.py")}),
                "read 10 lines",
            )
        })
        .collect();
    turn(steps)
}

/// The facts the pane paints with: one turn's time, and the layout the caller
/// names — compact, or full detail.
pub(super) fn facts(detail: bool) -> TranscriptFacts {
    TranscriptFacts {
        elapsed: vec![ELAPSED],
        detail,
        ..TranscriptFacts::default()
    }
}

/// The rows `turn` paints at `width × height`, one string per row.
pub(super) fn rows_in(
    turn: &TurnView,
    facts: &TranscriptFacts,
    width: usize,
    height: usize,
) -> Vec<String> {
    rendered(&transcript_rows(
        std::slice::from_ref(turn),
        facts,
        width,
        height,
    ))
}

/// The step lines of `rows`: the rows under the block's own gutter, each of
/// which a step's mark opens.
pub(super) fn step_lines(rows: &[String]) -> Vec<&String> {
    rows.iter()
        .filter(|row| row.starts_with("    ") && row.trim_start().starts_with("✓ "))
        .collect()
}

/// Every step row of `rows`, in reading order: the lines a step paints, at any
/// marking, under the block's gutter.
pub(super) fn step_rows(rows: &[String]) -> Vec<&String> {
    rows.iter().filter(|row| row.starts_with("    ")).collect()
}
