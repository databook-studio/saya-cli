use super::super::*;

/// An already-sent tool message — a result the model has already seen in an
/// earlier turn — must never be rewritten by the budget trim. Rewriting it
/// would feed the model different text than it saw before, worse than
/// dropping the oldest context. Only the newest group (the turn not yet
/// sent at the point of trimming) may be truncated.
///
/// The conversation here carries a large, already-sent tool result that is
/// the largest `tool` message, alongside the current turn's group with a
/// small result. The budget is exceeded. The trim must leave the sent
/// result byte-identical and still bring the conversation under the budget
/// by truncating the newest (unsent) result.
#[test]
fn trim_never_mutates_an_already_sent_tool_message() {
    let old_sent = "X".repeat(200);
    let new_result = "Y".repeat(120);
    let mut messages = vec![
        crate::ChatMessage::text("system", "s"),
        crate::ChatMessage::text("user", "ask"),
        // An already-sent tool result from an earlier turn. It has no
        // preceding assistant tool-call turn in this slice, so it is not
        // part of the newest group — it is exactly the kind of message the
        // trim must not touch.
        crate::ChatMessage {
            role: "tool".into(),
            content: old_sent.clone(),
            tool_calls: Vec::new(),
            tool_call_id: Some("c-old".into()),
        },
        // The newest group — the current turn's call, not yet sent.
        crate::ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![crate::ToolCall {
                id: "c1".into(),
                name: "t".into(),
                arguments: serde_json::json!({}),
            }],
            tool_call_id: None,
        },
        crate::ChatMessage {
            role: "tool".into(),
            content: new_result.clone(),
            tool_calls: Vec::new(),
            tool_call_id: Some("c1".into()),
        },
    ];
    let total: usize = messages.iter().map(message_size).sum();
    let newest_tool = message_size(&messages[4]);
    let everything_else = total - newest_tool;
    // Budget: over the total so the trim acts, but with enough headroom that
    // truncating only the newest (unsent) result can bring it back under.
    let budget = everything_else + TRUNCATED_MARKER.len() + 10;
    assert!(
        budget < total,
        "test setup must exceed the budget: {budget} vs {total}"
    );
    trim_to_budget(&mut messages, budget, 2);
    assert_eq!(
        messages[2].content, old_sent,
        "an already-sent tool message must not be mutated"
    );
    let after: usize = messages.iter().map(message_size).sum();
    assert!(
        after <= budget,
        "the budget must still bind after trim: {after} vs {budget}"
    );
}

/// A one-message tool-result group: the assistant turn that issued the call
/// plus its `tool` result, built from a SQL statement and a result value.
fn group(call_id: &str, sql: &str, result: serde_json::Value) -> Vec<crate::ChatMessage> {
    vec![
        crate::ChatMessage {
            role: "assistant".into(),
            content: String::new(),
            tool_calls: vec![crate::ToolCall {
                id: call_id.into(),
                name: "bounded_sql_query".into(),
                arguments: serde_json::json!({"sql": sql}),
            }],
            tool_call_id: None,
        },
        crate::ChatMessage {
            role: "tool".into(),
            content: serde_json::to_string(&result).expect("result serializes"),
            tool_calls: Vec::new(),
            tool_call_id: Some(call_id.into()),
        },
    ]
}

/// Under the budget, the conversation is left untouched — byte-identical,
/// not silently reorganized. (Part 3: compaction only fires when over.)
#[test]
fn trim_under_budget_leaves_the_conversation_untouched() {
    let mut messages = vec![
        crate::ChatMessage::text("system", "sys"),
        crate::ChatMessage::text("user", "ask"),
    ];
    messages.extend(group(
        "c0",
        "SELECT 1",
        serde_json::json!({"columns":["a"],"rows":[[1]],"row_count":1,"truncated":false,"executed_sql":"SELECT 1"}),
    ));
    let snapshot = messages.clone();
    let total: usize = messages.iter().map(message_size).sum();
    trim_to_budget(&mut messages, total + 1024, 2);
    assert_eq!(
        messages, snapshot,
        "under-budget trim must not touch the conversation"
    );
}

/// Over the budget, one compaction evicts the oldest tool-result groups and
/// replaces them with a digest, keeping the system message and the newest
/// group byte-identical. The newest group is the freshest context; the
/// system message carries policy. Neither is rewritten.
#[test]
fn compaction_keeps_the_system_message_and_newest_group_byte_identical() {
    let big = serde_json::json!({"columns":["a"],"rows":[["B".repeat(4000)]],"row_count":1,"truncated":false,"executed_sql":"SELECT a FROM x"});
    let small = serde_json::json!({"columns":["c"],"rows":[["ok"]],"row_count":1,"truncated":false,"executed_sql":"SELECT c FROM z"});
    let mut messages = vec![
        crate::ChatMessage::text("system", "sys"),
        crate::ChatMessage::text("user", "ask"),
    ];
    messages.extend(group("c0", "SELECT a FROM x", big.clone()));
    messages.extend(group("c1", "SELECT b FROM y", big));
    messages.extend(group("c2", "SELECT c FROM z", small));
    let system_snapshot = messages[0].clone();
    let newest_snapshot: Vec<crate::ChatMessage> = messages[messages.len() - 2..].to_vec();
    // Budget large enough that evicting the two big groups with a digest
    // fits while keeping the newest group intact, but too small to keep any
    // of the big groups (each dwarfs the allowance).
    let head: usize = messages[..2].iter().map(message_size).sum();
    let newest_size: usize = messages[messages.len() - 2..]
        .iter()
        .map(message_size)
        .sum();
    let budget = head + newest_size + 1024;
    trim_to_budget(&mut messages, budget, 2);
    assert_eq!(
        messages[0], system_snapshot,
        "the system message must be byte-identical after compaction"
    );
    let tail: Vec<crate::ChatMessage> = messages[messages.len() - 2..].to_vec();
    assert_eq!(
        tail, newest_snapshot,
        "the newest group must be byte-identical after compaction"
    );
    let after: usize = messages.iter().map(message_size).sum();
    assert!(after <= budget, "the budget must bind: {after} vs {budget}");
}

/// The compaction digest names the statements it replaced and carries no
/// row value — only the statement text and the result's shape (row count
/// and column names). A planted cell value in an evicted result must not
/// survive into the digest.
#[test]
fn compaction_digest_names_statements_and_carries_no_row_value() {
    let planted = "PLANTED_CELL_SENTINEL_42";
    // The evicted result is large (a big cell value) so a budget that
    // keeps the newest group intact genuinely exceeds the total and fires
    // a compaction. The planted sentinel lives in a row VALUE.
    let evicted_result = serde_json::json!({
        "columns": ["id", "secret"],
        "rows": [[1, format!("{planted}{}", "P".repeat(3000))]],
        "row_count": 1,
        "truncated": false,
        "executed_sql": "SELECT id, secret FROM t"
    });
    let newest_result = serde_json::json!({
        "columns": ["one"], "rows": [[1]], "row_count": 1, "truncated": false, "executed_sql": "SELECT 1"
    });
    let mut messages = vec![
        crate::ChatMessage::text("system", "sys"),
        crate::ChatMessage::text("user", "ask"),
    ];
    messages.extend(group("c0", "SELECT id, secret FROM t", evicted_result));
    messages.extend(group("c1", "SELECT 1", newest_result));
    let head: usize = messages[..2].iter().map(message_size).sum();
    let newest_size: usize = messages[messages.len() - 2..]
        .iter()
        .map(message_size)
        .sum();
    // Budget keeps the newest group intact (head + newest + allowance) but
    // is far below the total, so the large evicted group is compacted.
    let budget = head + newest_size + 256;
    let total: usize = messages.iter().map(message_size).sum();
    assert!(
        budget < total,
        "test setup must exceed the budget: {budget} vs {total}"
    );
    trim_to_budget(&mut messages, budget, 2);
    let digest = messages
        .iter()
        .find(|message| crate::loop_runner::compaction::is_digest(message))
        .expect("a digest must replace the evicted group");
    assert!(
        digest.content.contains("SELECT id, secret FROM t"),
        "the digest must name the evicted statement: {}",
        digest.content
    );
    assert!(
        !digest.content.contains(planted),
        "a row value must never appear in the digest: {}",
        digest.content
    );
    let after: usize = messages.iter().map(message_size).sum();
    assert!(after <= budget, "the budget must bind: {after} vs {budget}");
}
