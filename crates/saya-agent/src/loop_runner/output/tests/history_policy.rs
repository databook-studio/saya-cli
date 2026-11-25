//! The history policy's budget: what is replayed whole, what a pass may
//! replace, and how often a pass rewrites the prefix.

use super::super::history_stub::*;
use crate::{ChatMessage, ToolCall};
use serde_json::{Value, json};

/// A budget small enough that the fixtures below cross it, and a hard ceiling
/// high enough that only the hard-ceiling test reaches the last resort.
const BUDGET: usize = 8_000;
const HARD: usize = 100_000;
const RESULT_BYTES: usize = 4_000;

fn group(index: usize, name: &str, arguments: Value, result: Value) -> Vec<ChatMessage> {
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

fn read_group(index: usize, path: &str, start_line: u64, line_count: u64) -> Vec<ChatMessage> {
    group(
        index,
        "read",
        json!({"path": path, "start_line": start_line, "line_count": line_count}),
        json!({
            "path": path,
            "start_line": start_line,
            "lines_returned": line_count,
            "content": "x".repeat(RESULT_BYTES),
        }),
    )
}

fn shell_group(index: usize, command: &str) -> Vec<ChatMessage> {
    group(
        index,
        "shell",
        json!({"command": command}),
        json!({"command": command, "exit_code": 0, "stdout": "x".repeat(RESULT_BYTES)}),
    )
}

fn write_group(index: usize, path: &str) -> Vec<ChatMessage> {
    group(
        index,
        "write",
        json!({"path": path, "content": "done"}),
        json!({"path": path, "size": 4, "digest": "abc"}),
    )
}

fn base() -> Vec<ChatMessage> {
    vec![
        ChatMessage::text("system", "s"),
        ChatMessage::text("user", "ask"),
    ]
}

/// The tool message of group `index` in a conversation whose groups start at
/// message 2.
fn tool_at(index: usize) -> usize {
    3 + 2 * index
}

fn is_stub(message: &ChatMessage) -> bool {
    message.content.starts_with(STUB_SENTINEL)
}

fn conversation(groups: impl IntoIterator<Item = Vec<ChatMessage>>) -> Vec<ChatMessage> {
    let mut messages = base();
    for group in groups {
        messages.extend(group);
    }
    messages
}

#[test]
fn a_history_under_its_budget_is_byte_identical() {
    let mut messages = conversation(
        (0..12).map(|index| read_group(index, &format!("src/file_{index}.rs"), 1, 200)),
    );
    let snapshot = messages.clone();
    let mut state = HistoryStubState::default();
    assert_eq!(
        stub_with_limits(&mut messages, 2, usize::MAX, usize::MAX, &mut state),
        0,
        "nothing is stubbed below the budget"
    );
    assert_eq!(messages, snapshot, "the request is byte-identical");
}

#[test]
fn the_latest_read_of_a_path_is_never_stubbed_even_many_groups_later() {
    let mut messages = base();
    messages.extend(read_group(0, "src/a.rs", 1, 200));
    for index in 1..=10 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    let latest = messages[tool_at(0)].content.clone();
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state);
    assert_eq!(
        messages[tool_at(0)].content,
        latest,
        "the latest read of a path is the model's working copy"
    );
    assert!(!is_stub(&messages[tool_at(0)]));
}

#[test]
fn a_superseded_read_is_stubbed_and_names_the_later_call() {
    let mut messages = base();
    messages.extend(read_group(0, "src/a.rs", 1, 200));
    messages.extend(read_group(1, "src/a.rs", 1, 200));
    for index in 2..=9 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state);
    let stub = &messages[tool_at(0)].content;
    assert!(is_stub(&messages[tool_at(0)]), "{stub}");
    assert!(
        stub.contains("superseded by"),
        "the stub names the cause: {stub}"
    );
    assert!(stub.contains("call-1"), "the later call is named: {stub}");
    assert!(
        !stub.contains("call read again") && !stub.contains("re-read:"),
        "a superseded stub never tells the model to call again: {stub}"
    );
    assert!(
        !is_stub(&messages[tool_at(1)]),
        "the later read is the working copy and stays whole"
    );
}

#[test]
fn a_repeated_shell_call_is_superseded() {
    let mut messages = base();
    messages.extend(shell_group(0, "cargo test"));
    for index in 1..=8 {
        messages.extend(shell_group(
            index,
            &format!("cargo build -p module_{index}"),
        ));
    }
    messages.extend(shell_group(9, "cargo test"));
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state);
    assert!(is_stub(&messages[tool_at(0)]));
    assert!(messages[tool_at(0)].content.contains("superseded by"));
    assert!(
        !is_stub(&messages[tool_at(9)]),
        "the later identical call is the live result"
    );
}

#[test]
fn a_later_write_supersedes_an_earlier_read() {
    let mut messages = base();
    messages.extend(read_group(0, "src/a.rs", 1, 200));
    messages.extend(write_group(1, "src/a.rs"));
    for index in 2..=9 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state);
    let stub = &messages[tool_at(0)].content;
    assert!(is_stub(&messages[tool_at(0)]), "{stub}");
    assert!(stub.contains("superseded by"), "{stub}");
    assert!(stub.contains("write"), "the later write is named: {stub}");
}

#[test]
fn a_narrower_later_read_does_not_supersede() {
    let mut messages = base();
    messages.extend(read_group(0, "src/a.rs", 1, 200));
    messages.extend(read_group(1, "src/a.rs", 1, 50));
    for index in 2..=9 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state);
    assert!(
        !is_stub(&messages[tool_at(0)]),
        "a later narrower read leaves the wider result needed"
    );
}

#[test]
fn old_non_read_output_is_stubbed_only_outside_the_last_six_groups() {
    let mut messages = conversation(
        (0..10).map(|index| shell_group(index, &format!("cargo test -p module_{index}"))),
    );
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, HARD, &mut state);
    for index in 0..4 {
        assert!(is_stub(&messages[tool_at(index)]), "group {index} is old");
    }
    for index in 4..10 {
        assert!(
            !is_stub(&messages[tool_at(index)]),
            "group {index} is inside the last six groups"
        );
    }
}

#[test]
fn a_latest_read_is_stubbed_only_above_the_hard_ceiling_and_says_to_re_read() {
    let mut messages = conversation(
        (0..20).map(|index| read_group(index, &format!("src/file_{index}.rs"), 1, 200)),
    );
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, BUDGET, 20_000, &mut state);
    let stubs = (0..20)
        .filter(|&index| is_stub(&messages[tool_at(index)]))
        .count();
    assert!(stubs > 0, "above the hard ceiling reads may be stubbed");
    let stub = &messages[tool_at(0)].content;
    assert!(
        stub.contains("re-read the file before editing"),
        "the stub says the file must be re-read before editing: {stub}"
    );
}

#[test]
fn rewrites_are_batched_across_many_over_budget_requests() {
    let budget = 400_000;
    let hard = 1_000_000;
    let mut messages = base();
    let mut state = HistoryStubState::default();
    let mut previous = messages.clone();
    let mut rewrites = 0;
    for index in 0..40 {
        messages.extend(group(
            index,
            "shell",
            json!({"command": format!("cargo test -p module_{index}")}),
            json!({"stdout": "x".repeat(20_000)}),
        ));
        stub_with_limits(&mut messages, 2, budget, hard, &mut state);
        if messages[..previous.len()] != previous[..] {
            rewrites += 1;
        }
        previous = messages.clone();
    }
    println!("prefix rewrites over 40 over-budget requests: {rewrites}");
    assert!(
        rewrites <= 4,
        "a 40-request over-budget job rewrites its prefix rarely, not every request: {rewrites}"
    );
}

#[test]
fn a_settled_region_is_not_rewritten_when_a_later_call_supersedes_it() {
    let budget = 20_000;
    let hard = 1_000_000;
    let mut messages = base();
    messages.extend(read_group(0, "src/a.rs", 1, 200));
    for index in 1..=10 {
        messages.extend(shell_group(index, &format!("cargo test -p module_{index}")));
    }
    let mut state = HistoryStubState::default();
    stub_with_limits(&mut messages, 2, budget, hard, &mut state);
    let settled = messages[tool_at(0)].content.clone();
    assert!(!is_stub(&messages[tool_at(0)]));

    // A later read supersedes the settled one; enough new output follows to
    // cross the budget again. The settled prefix must not be rewritten.
    messages.extend(read_group(11, "src/a.rs", 1, 200));
    for index in 12..=25 {
        messages.extend(shell_group(
            index,
            &format!("cargo build -p module_{index}"),
        ));
    }
    stub_with_limits(&mut messages, 2, budget, hard, &mut state);
    assert_eq!(
        messages[tool_at(0)].content,
        settled,
        "a settled message is not rewritten by a later pass"
    );
}

#[test]
fn a_pass_below_the_budget_builds_no_group_list() {
    let mut messages = conversation(
        (0..8).map(|index| read_group(index, &format!("src/file_{index}.rs"), 1, 200)),
    );
    work::reset();
    let mut state = HistoryStubState::default();
    assert_eq!(
        stub_with_limits(&mut messages, 2, usize::MAX, usize::MAX, &mut state),
        0
    );
    assert_eq!(
        work::taken(),
        0,
        "a pass with nothing to do allocates no group list"
    );
}
