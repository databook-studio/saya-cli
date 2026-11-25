use super::super::history_stub::*;
use crate::{ChatMessage, ToolCall};
use serde_json::json;

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

fn read_result(index: usize) -> serde_json::Value {
    let content = (1..=200)
        .map(|line| format!("line {line:03} of file {index}"))
        .collect::<Vec<_>>()
        .join("\n");
    json!({
        "path": format!("src/file_{index}.rs"),
        "start_line": 1,
        "lines_returned": 200,
        "content": content,
    })
}

fn read_group(index: usize) -> Vec<ChatMessage> {
    group(
        index,
        "read",
        json!({"path": format!("src/file_{index}.rs"), "start_line": 1, "line_count": 200}),
        read_result(index),
    )
}

fn conversation(groups: usize) -> Vec<ChatMessage> {
    let mut messages = vec![
        ChatMessage::text("system", "s"),
        ChatMessage::text("user", "ask"),
    ];
    for index in 0..groups {
        messages.extend(read_group(index));
    }
    messages
}

/// The tool message of group `index` in a conversation built by
/// [`conversation`].
fn tool_at(index: usize) -> usize {
    3 + 2 * index
}

#[test]
fn old_tool_results_become_bounded_stubs_and_the_newest_turns_stay_whole() {
    let mut messages = conversation(5);
    let newest = messages[tool_at(4)].content.clone();
    let before = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();
    let stubbed = stub_old_tool_results(&mut messages, 2);
    assert_eq!(stubbed, 3, "three groups are older than the last two");
    for index in 0..3 {
        assert!(
            messages[tool_at(index)].content.starts_with(STUB_SENTINEL),
            "group {index} is stubbed"
        );
        assert!(
            messages[tool_at(index)].content.len() <= MAX_STUB_BYTES,
            "stub {index} is bounded"
        );
    }
    for index in 3..5 {
        assert!(
            !messages[tool_at(index)].content.starts_with(STUB_SENTINEL),
            "group {index} stays whole"
        );
    }
    assert_eq!(
        messages[tool_at(4)].content,
        newest,
        "the newest result is whole"
    );
    let after = messages
        .iter()
        .map(|message| message.content.len())
        .sum::<usize>();
    assert!(
        after < before / 2,
        "stubbing must cut the history substantially: {before} -> {after}"
    );
}

#[test]
fn stubbing_is_idempotent_and_the_stubbed_prefix_stays_byte_stable() {
    let mut messages = conversation(6);
    assert!(stub_old_tool_results(&mut messages, 2) > 0);
    let snapshot = messages.clone();
    assert_eq!(
        stub_old_tool_results(&mut messages, 2),
        0,
        "a second pass rewrites nothing"
    );
    assert_eq!(messages, snapshot, "the stubbed prefix is byte-identical");
}

#[test]
fn a_stub_keeps_tool_arguments_outcome_size_and_first_and_last_lines() {
    let mut messages = conversation(3);
    stub_old_tool_results(&mut messages, 2);
    let stub = &messages[tool_at(0)].content;
    assert!(stub.contains("tool: read"), "{stub}");
    assert!(stub.contains("src/file_0.rs"), "arguments survive: {stub}");
    assert!(stub.contains("outcome: ok"), "{stub}");
    assert!(
        stub.contains("bytes"),
        "the original size is stated: {stub}"
    );
    assert!(stub.contains("first: line 001 of file 0"), "{stub}");
    assert!(stub.contains("last: line 200 of file 0"), "{stub}");
}

#[test]
fn a_stub_names_the_tool_and_arguments_so_the_model_can_re_read() {
    let mut messages = conversation(3);
    stub_old_tool_results(&mut messages, 2);
    let stub = &messages[tool_at(0)].content;
    assert!(
        stub.contains("re-read: call read again with the same arguments"),
        "{stub}"
    );
    assert!(
        stub.contains("\"path\":\"src/file_0.rs\""),
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
        json!({"exit_code": 0, "stdout": "api_key=SUPERSECRETVALUE"}),
    ));
    messages.extend(read_group(1));
    messages.extend(read_group(2));
    stub_old_tool_results(&mut messages, 2);
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
        json!({"error": "relation does not exist"}),
    ));
    messages.extend(read_group(1));
    messages.extend(read_group(2));
    stub_old_tool_results(&mut messages, 2);
    let stub = &messages[tool_at(0)].content;
    assert!(stub.contains("outcome: error"), "{stub}");
    assert!(
        stub.contains("re-read: run the same statement again with bounded_sql_query"),
        "{stub}"
    );
}

#[test]
fn a_short_conversation_is_left_byte_identical() {
    let mut messages = conversation(WHOLE_TOOL_TURNS);
    let snapshot = messages.clone();
    assert_eq!(stub_old_tool_results(&mut messages, 2), 0);
    assert_eq!(messages, snapshot);
}
