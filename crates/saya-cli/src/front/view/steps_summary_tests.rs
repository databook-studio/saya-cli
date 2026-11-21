//! The folded summary of an activity block: the one line a successful turn
//! collapses to, built from the block's steps and the time it took.

use std::time::Duration;

use serde_json::{Value, json};

use super::{Step, StepState, folded_summary};

/// A step that finished with `summary` as its outcome.
fn step(tool: &str, arguments: Value, summary: &str) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state: StepState::Done {
            summary: summary.to_owned(),
        },
    }
}

/// The demo turn's block — a schema read and three queries — folds to exactly
/// the line the screen design draws.
#[test]
fn the_demo_block_folds_to_its_one_summary_line() {
    let steps = [
        step(
            "schema_discovery",
            json!({"connection": "demo"}),
            "discovered 12 tables in demo",
        ),
        step(
            "bounded_sql_query",
            json!({"connection": "demo", "sql": "SELECT status, COUNT(*) FROM orders"}),
            "3 rows",
        ),
        step(
            "bounded_sql_query",
            json!({"connection": "demo", "sql": "SELECT order_date FROM orders"}),
            "35 rows",
        ),
        step(
            "bounded_sql_query",
            json!({"connection": "demo", "sql": "SELECT COUNT(*) FROM orders"}),
            "1 row",
        ),
    ];
    assert_eq!(
        folded_summary(&steps, Duration::from_secs(14)),
        "4 steps · read schema, 3 queries · 14s",
        "the folded line counts the steps, names their kinds and states the time"
    );
}

/// A block of one step reads in the singular, and the elapsed time picks its
/// unit: seconds under a minute, minutes and seconds above it.
#[test]
fn one_step_and_a_longer_elapsed_read_in_their_own_units() {
    let steps = [step("read", json!({"path": "pricing.py"}), "read 40 lines")];
    assert_eq!(
        folded_summary(&steps, Duration::from_secs(4)),
        "1 step · 1 read · 4s",
        "one step is not \"1 steps\""
    );
    assert_eq!(
        folded_summary(&steps, Duration::from_secs(90)),
        "1 step · 1 read · 1m 30s",
        "past a minute the time states minutes and seconds"
    );
}

/// Kinds keep the order they first appear in, so the line reads as the block
/// did rather than as a map's keys happen to sort.
#[test]
fn kinds_keep_the_order_the_block_first_reached_them() {
    let steps = [
        step(
            "shell",
            json!({"command": "python3 -m unittest"}),
            "python3 exited 0",
        ),
        step("edit", json!({"path": "pricing.py"}), "pricing.py edited"),
        step("shell", json!({"command": "pytest"}), "pytest exited 0"),
    ];
    assert_eq!(
        folded_summary(&steps, Duration::from_secs(2)),
        "3 steps · 2 commands, 1 edit · 2s",
        "the first kind reached leads the list"
    );
}
