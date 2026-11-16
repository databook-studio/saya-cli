//! The failure-key behavior tests: the keying rules for SQL-carrying and
//! non-SQL calls, and the bounded canonical rendering of arguments.

use super::*;

fn call_with(name: &str, arguments: serde_json::Value) -> ToolCall {
    ToolCall {
        id: "c1".into(),
        name: name.into(),
        arguments,
    }
}

/// Two calls whose argument objects were built with the same keys in
/// different orders are the same call: the canonical rendering sorts
/// object keys recursively, so the keys collide.
#[test]
fn arguments_built_in_different_key_orders_produce_one_key() {
    let first = call_with(
        "run_program",
        serde_json::json!({"command": "ls", "flags": {"all": true, "human": true}, "cwd": "/tmp"}),
    );
    let second = call_with(
        "run_program",
        serde_json::json!({"cwd": "/tmp", "flags": {"human": true, "all": true}, "command": "ls"}),
    );
    assert_eq!(key_of(&first), key_of(&second));
}

/// A changed argument value is a different call — canonicalisation of the
/// serialization must not blur distinct content.
#[test]
fn a_changed_argument_value_is_a_different_key() {
    let first = call_with("run_program", serde_json::json!({"command": "ls"}));
    let second = call_with("run_program", serde_json::json!({"command": "ls -la"}));
    assert_ne!(key_of(&first), key_of(&second));
}

/// The tool name participates in the key: the same arguments under a
/// different tool are a different invocation.
#[test]
fn the_tool_name_participates_in_the_key() {
    let first = call_with("run_program", serde_json::json!({"command": "ls"}));
    let second = call_with("other_tool", serde_json::json!({"command": "ls"}));
    assert_ne!(key_of(&first), key_of(&second));
}

/// The SQL path wins: a call carrying a `sql` string keys on the
/// statement alone, so a different companion argument does not change the
/// key (a known-bad statement must not look fresh on another connection).
#[test]
fn a_sql_carrying_call_keys_on_the_statement_alone() {
    let first = call_with(
        "bounded_sql_query",
        serde_json::json!({"sql": "SELECT bad", "connection": "one"}),
    );
    let second = call_with(
        "bounded_sql_query",
        serde_json::json!({"connection": "two", "sql": "SELECT bad"}),
    );
    assert_eq!(key_of(&first), key_of(&second));
    assert_eq!(key_of(&first), CallKey::Sql("SELECT bad".into()));
}

/// A SQL statement and a tool call whose arguments spell alike do not
/// collide — the variants keep the two paths apart by construction.
#[test]
fn a_sql_statement_and_a_tool_call_spelled_alike_do_not_collide() {
    assert_ne!(
        CallKey::Sql("SELECT 1".into()),
        CallKey::ToolCall("run_program".into(), "SELECT 1".into()),
    );
}

/// Huge arguments are bounded: identical huge calls still collide, the
/// rendering never exceeds the prefix plus the hash suffix, and calls
/// differing only past the prefix are told apart by the hash.
#[test]
fn a_huge_arguments_key_is_bounded_and_still_identifies_its_call() {
    let blob = "y".repeat(4_000);
    let first = key_of(&call_with(
        "run_program",
        serde_json::json!({"blob": blob.clone()}),
    ));
    let second = key_of(&call_with("run_program", serde_json::json!({"blob": blob})));
    assert_eq!(first, second, "identical huge arguments collide");
    let CallKey::ToolCall(_, ref rendering) = first else {
        panic!("a non-SQL call keys as a tool call");
    };
    // A prefix of at most MAX_ARGUMENTS_BYTES bytes, plus "#fnv1a:" and
    // 16 hex digits.
    assert!(
        rendering.len() <= MAX_ARGUMENTS_BYTES + 23,
        "the rendering is bounded, got {} bytes",
        rendering.len()
    );
    assert!(rendering.contains("#fnv1a:"));
    let truncated = key_of(&call_with(
        "run_program",
        serde_json::json!({"blob": format!("{}z", "y".repeat(4_000))}),
    ));
    assert_ne!(
        first, truncated,
        "a call differing past the prefix must not collide"
    );
}

/// The truncation prefix must never split a UTF-8 sequence: a rendering
/// whose 512th byte falls inside a multi-byte character still produces a
/// key (and the same arguments produce the same one).
#[test]
fn the_truncation_prefix_never_splits_a_code_point() {
    let blob = "€".repeat(1_000);
    let first = key_of(&call_with(
        "run_program",
        serde_json::json!({"blob": blob.clone()}),
    ));
    let second = key_of(&call_with("run_program", serde_json::json!({"blob": blob})));
    assert_eq!(first, second, "identical non-ASCII arguments collide");
}
