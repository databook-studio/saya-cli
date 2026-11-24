//! The verb map: every row of the activity block's map reads as the short
//! phrase the screen design draws, and a workspace `cd` never reaches the line.

use std::path::Path;

use serde_json::{Value, json};

use super::{Step, StepState, step_line};

/// A step as the reducer holds one: the tool the model called, its raw
/// arguments, and the state it stands in.
fn step(tool: &str, arguments: Value, state: StepState) -> Step {
    Step {
        tool: tool.to_owned(),
        arguments,
        state,
    }
}

/// A step that finished with `summary` as its outcome.
fn done(summary: &str) -> StepState {
    StepState::Done {
        summary: summary.to_owned(),
    }
}

/// One case per row of the map: the tool the model calls, the arguments the
/// call carries, and the line the block draws for it.
#[test]
fn every_tool_reads_as_its_own_verb() {
    let cases: [(&str, Value, &str); 20] = [
        (
            "schema_discovery",
            json!({"connection": "demo"}),
            "Read schema",
        ),
        (
            "bounded_sql_query",
            json!({"connection": "demo", "sql": "SELECT COUNT(*) FROM orders"}),
            "Query demo",
        ),
        (
            "bounded_sql_query_all",
            json!({"sql": "SELECT 1"}),
            "Query all databases",
        ),
        (
            "result_shape",
            json!({"connection": "demo", "sql": "SELECT 1"}),
            "Check data",
        ),
        (
            "column_health",
            json!({"connection": "demo", "sql": "SELECT 1"}),
            "Check data",
        ),
        (
            "join_check",
            json!({"connection": "demo", "sql": "SELECT 1"}),
            "Check data",
        ),
        ("contract_search", json!({"query": "refunds"}), "Recall"),
        (
            "contract_read",
            json!({"connection": "demo", "table": "orders"}),
            "Recall",
        ),
        (
            "designate_answer",
            json!({"sql": "SELECT COUNT(*) FROM orders"}),
            "Mark answer",
        ),
        ("read", json!({"path": "pricing.py"}), "Read pricing.py"),
        ("list", json!({"path": "."}), "Search"),
        ("glob", json!({"pattern": "**/*.py"}), "Search"),
        ("grep", json!({"pattern": "discount"}), "Search"),
        ("write", json!({"path": "pricing.py"}), "Edit pricing.py"),
        ("edit", json!({"path": "pricing.py"}), "Edit pricing.py"),
        (
            "shell",
            json!({"command": "python3 -m unittest"}),
            "Run python3 -m unittest",
        ),
        (
            "web_fetch",
            json!({"url": "https://docs.rs/serde/latest/serde/"}),
            "Fetch docs.rs",
        ),
        (
            "web_search",
            json!({"query": "sqlite upsert"}),
            "Search web",
        ),
        (
            "helpers",
            json!({"description": "explore-1", "prompt": "find the pricing tests"}),
            "Helper explore-1",
        ),
        ("some_future_tool", Value::Null, "some_future_tool"),
    ];
    for (tool, arguments, expected) in cases {
        assert_eq!(
            step_line(&step(tool, arguments, StepState::Running), None),
            expected,
            "{tool} reads as {expected:?}"
        );
    }
}

/// A settled query states how many rows it returned, so its line carries the
/// count; a schema read's outcome is prose and stays off the line.
#[test]
fn a_settled_query_line_carries_the_rows_it_returned() {
    let query = step(
        "bounded_sql_query",
        json!({"connection": "demo", "sql": "SELECT status FROM orders"}),
        done("3 rows"),
    );
    assert_eq!(
        step_line(&query, None),
        "Query demo · 3 rows",
        "a settled query names the rows the operator waits for"
    );
    let schema = step(
        "schema_discovery",
        json!({"connection": "demo"}),
        done("discovered 12 tables in demo"),
    );
    assert_eq!(
        step_line(&schema, None),
        "Read schema",
        "the schema read stays its verb alone"
    );
}

/// A command run from the workspace root is shown as the command: the `cd`
/// that only exists to reach the root is not the operator's business.
#[test]
fn a_shell_line_strips_the_workspace_cd_prefix() {
    let root = Path::new("/tmp/x/work");
    let call = step(
        "shell",
        json!({"command": "cd /tmp/x/work && python3 -m unittest"}),
        StepState::Running,
    );
    assert_eq!(
        step_line(&call, Some(root)),
        "Run python3 -m unittest",
        "the cd into the tool's own root is not part of the command"
    );
}

/// A `cd` somewhere else is the command, and stays — dropping it would claim
/// the call ran where it did not.
#[test]
fn a_cd_into_another_directory_is_part_of_the_command() {
    let root = Path::new("/tmp/x/work");
    let call = step(
        "shell",
        json!({"command": "cd /tmp/x/other && python3 -m unittest"}),
        StepState::Running,
    );
    assert_eq!(
        step_line(&call, Some(root)),
        "Run cd /tmp/x/other && python3 -m unittest",
        "only the workspace root's own cd is stripped"
    );
}
