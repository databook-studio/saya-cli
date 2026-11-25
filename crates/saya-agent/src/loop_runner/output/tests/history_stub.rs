//! The stub itself: what it keeps, what it redacts, and that it is one-way.

use super::super::history_stub::*;
use crate::{ChatMessage, ToolCall};
use serde_json::json;

const BUDGET: usize = 8_000;
const HARD: usize = 1_000_000;

fn group(
    index: usize,
    name: &str,
    arguments: serde_json::Value,
    result: serde_json::Value,
) -> Vec<ChatMessage> {
    vec![
        ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![ToolCall {
                id: format!("call-{index}"),
                name: name.into(),
                arguments,
            }],
            ..Default::default()
        },
        ChatMessage {
            role: "tool".into(),
            content: serde_json::to_string(&result).expect("result serializes"),
            tool_call_id: Some(format!("call-{index}")),
            ..Default::default()
        },
    ]
}

fn shell_group(index: usize, command: &str) -> Vec<ChatMessage> {
    group(
        index,
        "shell",
        json!({"command": command}),
        json!({"command": command, "exit_code": 0, "stdout": "x".repeat(4_000)}),
    )
}

fn conversation(groups: usize) -> Vec<ChatMessage> {
    let mut messages = vec![
        ChatMessage::text("system", "s"),
        ChatMessage::text("user", "ask"),
    ];
    for index in 0..groups {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    messages
}

/// The tool message of group `index` in a conversation whose groups start at
/// message 2.
fn tool_at(index: usize) -> usize {
    3 + 2 * index
}

fn apply(messages: &mut [ChatMessage]) -> usize {
    stub_with_limits(messages, 2, BUDGET, HARD, &mut HistoryStubState::default())
}

#[test]
fn old_non_read_results_become_bounded_stubs() {
    let mut messages = conversation(10);
    let before = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();
    let stubbed = apply(&mut messages);
    assert_eq!(stubbed, 4, "the four oldest groups outside the last six");
    for index in 0..4 {
        assert!(
            messages[tool_at(index)].content.starts_with(STUB_SENTINEL),
            "group {index} is stubbed"
        );
        assert!(
            messages[tool_at(index)].content.len() <= MAX_STUB_BYTES,
            "stub {index} is bounded"
        );
    }
    let after = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();
    assert!(
        before.saturating_sub(after) > 4 * 3_000,
        "each stub must cut its result substantially: {before} -> {after}"
    );
}

#[test]
fn stubbing_is_idempotent_and_the_stubbed_prefix_stays_byte_stable() {
    let mut messages = conversation(10);
    let mut state = HistoryStubState::default();
    assert!(stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state) > 0);
    let snapshot = messages.clone();
    assert_eq!(
        stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state),
        0,
        "a second pass rewrites nothing"
    );
    assert_eq!(messages, snapshot, "the stubbed prefix is byte-identical");
}

#[test]
fn a_stub_keeps_tool_arguments_outcome_size_and_first_and_last_lines() {
    let mut messages = conversation(10);
    apply(&mut messages);
    let stub = &messages[tool_at(0)].content;
    assert!(stub.contains("tool: shell"), "{stub}");
    assert!(stub.contains("module_0"), "arguments survive: {stub}");
    assert!(stub.contains("outcome: ok"), "{stub}");
    assert!(
        stub.contains("bytes"),
        "the original size is stated: {stub}"
    );
    assert!(stub.contains("first: x"), "{stub}");
    assert!(stub.contains("last: x"), "{stub}");
}

#[test]
fn a_stub_names_the_tool_and_arguments_so_the_model_can_re_read() {
    let mut messages = conversation(10);
    apply(&mut messages);
    let stub = &messages[tool_at(0)].content;
    assert!(
        stub.contains("re-read: call shell again with the same arguments"),
        "{stub}"
    );
    assert!(
        stub.contains("cargo test -p module_0"),
        "the arguments to re-read are present: {stub}"
    );
}

#[test]
fn a_stub_carries_no_secret_shaped_value() {
    let mut messages = vec![
        ChatMessage::text("system", "s"),
        ChatMessage::text("user", "ask"),
    ];
    messages.extend(group(
        0,
        "shell",
        json!({"command": "psql postgres://admin:s3cr3t@db.internal/prod"}),
        json!({
            "exit_code": 0,
            "stdout": format!("api_key=SUPERSECRETVALUE\n{}", "connection ok\n".repeat(200)),
        }),
    ));
    for index in 1..=9 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    apply(&mut messages);
    let stub = &messages[tool_at(0)].content;
    assert!(
        stub.contains("[redacted]"),
        "the redaction marker is present: {stub}"
    );
    assert!(
        !stub.contains("SUPERSECRETVALUE"),
        "result secret leaked: {stub}"
    );
    assert!(!stub.contains("s3cr3t"), "argument secret leaked: {stub}");
}

#[test]
fn an_error_result_is_marked_as_an_error() {
    let mut messages = vec![
        ChatMessage::text("system", "s"),
        ChatMessage::text("user", "ask"),
    ];
    messages.extend(group(
        0,
        "bounded_sql_query",
        json!({"sql": "select 1"}),
        json!({"error": format!("relation does not exist: {}", "detail ".repeat(200))}),
    ));
    for index in 1..=9 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    apply(&mut messages);
    let stub = &messages[tool_at(0)].content;
    assert!(stub.contains("outcome: error"), "{stub}");
    assert!(
        stub.contains("re-read: run the same statement again with bounded_sql_query"),
        "{stub}"
    );
}

#[test]
fn a_result_no_larger_than_a_stub_is_kept_whole() {
    let mut messages = vec![
        ChatMessage::text("system", "s"),
        ChatMessage::text("user", "ask"),
    ];
    messages.extend(group(
        0,
        "checklist",
        json!({"tasks": 2}),
        json!({"tasks": 2}),
    ));
    for index in 1..=9 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    let small = messages[tool_at(0)].content.clone();
    apply(&mut messages);
    assert_eq!(
        messages[tool_at(0)].content,
        small,
        "a result no larger than a stub is cheaper whole"
    );
    assert!(messages[tool_at(1)].content.starts_with(STUB_SENTINEL));
}

#[test]
fn a_history_below_its_budget_is_left_byte_identical() {
    let mut messages = conversation(3);
    let snapshot = messages.clone();
    assert_eq!(
        stub_old_tool_results(&mut messages, 2, None, &mut HistoryStubState::default()),
        0
    );
    assert_eq!(messages, snapshot);
}

#[test]
fn a_pass_below_the_budget_builds_no_group_list() {
    let mut messages = conversation(8);
    work::reset();
    assert_eq!(
        stub_with_limits(
            &mut messages,
            2,
            usize::MAX,
            usize::MAX,
            &mut HistoryStubState::default()
        ),
        0
    );
    assert_eq!(
        work::taken(),
        0,
        "a pass with nothing to do allocates no group list"
    );
}
