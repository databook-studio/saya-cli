use super::*;
use crate::ToolCall;

#[test]
fn an_exact_fit_checkpoint_is_not_erased_by_a_trailing_separator() {
    let mut checkpoint = crate::loop_runner::coding_checkpoint::CodingCheckpoint::default();
    let arguments = serde_json::json!({"path":"file.rs"});
    let result = serde_json::json!({"path":"file.rs","digest":"a".repeat(64),"size":8});
    checkpoint.observe_serial(&crate::loop_runner::coding_checkpoint::observe(
        "read",
        &arguments,
        Ok(&result),
    ));
    let allowance = DIGEST_HEADER.len() + checkpoint.render(16 * 1024).len();
    let digest = build_digest_with_checkpoint(&[], &checkpoint, allowance);
    assert!(digest.content.len() <= allowance);
    assert!(
        digest.content.contains("file.rs"),
        "an exact-fit whole fact must survive: {}",
        digest.content
    );
    assert!(digest.content.contains(&"a".repeat(64)));
}

fn assistant_with_call(id: &str, sql: &str) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: "bounded_sql_query".into(),
            arguments: serde_json::json!({"sql": sql}),
        }],
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}

fn tool_result(id: &str, content: &str) -> ChatMessage {
    ChatMessage {
        role: "tool".into(),
        content: content.into(),
        tool_calls: Vec::new(),
        tool_call_id: Some(id.into()),
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}

/// The digest names each evicted statement and reports the result's row
/// count and column names — the shape, not the rows.
#[test]
fn digest_names_statements_and_shapes_without_row_values() {
    let result = serde_json::json!({
        "columns": ["id", "name"],
        "rows": [["a", "PLANTED_CELL_VALUE"], ["b", "c"]],
        "row_count": 2,
        "truncated": false,
        "executed_sql": "SELECT id, name FROM t"
    });
    let evicted = vec![
        assistant_with_call("c1", "SELECT id, name FROM t"),
        tool_result("c1", &serde_json::to_string(&result).unwrap()),
    ];
    let digest = build_digest(&evicted);
    assert!(is_digest(&digest));
    let body = &digest.content;
    assert!(
        body.contains("SELECT id, name FROM t"),
        "digest must name the statement: {body}"
    );
    assert!(
        body.contains("rows: 2"),
        "digest must report the row count: {body}"
    );
    assert!(
        body.contains("columns: [id, name]"),
        "digest must report column names: {body}"
    );
    assert!(
        !body.contains("PLANTED_CELL_VALUE"),
        "a row value must never appear in the digest: {body}"
    );
}

/// A truncated result (its content is no longer valid JSON) is recorded as
/// "result shape unavailable" — the statement is still named.
#[test]
fn digest_records_unavailable_shape_for_an_unparseable_result() {
    let evicted = vec![
        assistant_with_call("c1", "SELECT 1"),
        tool_result("c1", "{\"columns\":[\"a\"…[truncated]"),
    ];
    let digest = build_digest(&evicted);
    assert!(digest.content.contains("SELECT 1"));
    assert!(digest.content.contains("result shape unavailable"));
}

/// A non-row tool (no `sql`, no `row_count`/`columns`) still produces a
/// line, naming the absence of a statement.
#[test]
fn digest_handles_a_call_with_no_sql_argument() {
    let evicted = vec![ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: "c1".into(),
            name: "schema_discovery".into(),
            arguments: serde_json::json!({}),
        }],
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }];
    let digest = build_digest(&evicted);
    assert!(digest.content.contains("no sql statement"));
}

fn coding_call(id: &str, name: &str, arguments: serde_json::Value) -> ChatMessage {
    ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: id.into(),
            name: name.into(),
            arguments,
        }],
        tool_call_id: None,
        images: Vec::new(),
        reasoning: None,
        reasoning_signature: None,
    }
}

/// An evicted ranged read leaves its path and line range, and nothing else.
#[test]
fn an_evicted_read_records_its_path_and_line_range() {
    let evicted = vec![
        coding_call(
            "r1",
            "read",
            serde_json::json!({"path": "src/lib.rs", "start_line": 10, "line_count": 5}),
        ),
        tool_result(
            "r1",
            r#"{"path":"src/lib.rs","start_line":10,"line_count":5,"lines_returned":5,"content":"CANARY_READ_CONTENT"}"#,
        ),
    ];
    let digest = build_digest(&evicted);
    assert!(
        digest.content.contains("read src/lib.rs lines 10-14"),
        "digest must record the read's path and range: {}",
        digest.content
    );
    assert!(
        !digest.content.contains("CANARY_READ_CONTENT"),
        "read content must never enter the digest: {}",
        digest.content
    );
}

/// An unranged read records only its path.
#[test]
fn an_evicted_unranged_read_records_only_its_path() {
    let evicted = vec![coding_call(
        "r1",
        "read",
        serde_json::json!({"path": "src/lib.rs"}),
    )];
    let digest = build_digest(&evicted);
    assert!(digest.content.contains("read src/lib.rs"));
    assert!(!digest.content.contains("lines"));
}

/// Write, edit and delete record their path and numbers, never the bodies.
#[test]
fn evicted_write_edit_and_delete_record_paths_and_numbers() {
    let evicted = vec![
        coding_call(
            "w1",
            "write",
            serde_json::json!({"path": "src/new.rs", "content": "CANARY_WRITE_BODY"}),
        ),
        coding_call(
            "e1",
            "edit",
            serde_json::json!({"path": "src/lib.rs", "old_text": "CANARY_OLD", "new_text": "CANARY_NEW"}),
        ),
        tool_result("e1", r#"{"path":"src/lib.rs","replacements":3,"size":10}"#),
        coding_call("d1", "delete", serde_json::json!({"path": "src/gone.rs"})),
        tool_result("d1", r#"{"path":"src/gone.rs","removed":true}"#),
    ];
    let digest = build_digest(&evicted);
    let body = &digest.content;
    assert!(body.contains("wrote src/new.rs (17 bytes)"), "{body}");
    assert!(
        body.contains("edited src/lib.rs (3 replacements)"),
        "{body}"
    );
    assert!(body.contains("deleted src/gone.rs"), "{body}");
    for canary in ["CANARY_WRITE_BODY", "CANARY_OLD", "CANARY_NEW"] {
        assert!(!body.contains(canary), "{canary} leaked: {body}");
    }
}

/// A failed call keeps its line and says so.
#[test]
fn a_failed_coding_call_records_the_failed_marker() {
    let evicted = vec![
        coding_call(
            "r1",
            "read",
            serde_json::json!({"path": "src/lib.rs", "start_line": 10, "line_count": 5}),
        ),
        tool_result("r1", r#"{"error":"workspace read was cancelled"}"#),
    ];
    let digest = build_digest(&evicted);
    assert!(
        digest
            .content
            .contains("read src/lib.rs lines 10-14 (failed)"),
        "{}",
        digest.content
    );
}

/// At most 64 coding lines survive, with the rest counted.
#[test]
fn coding_lines_are_bounded_at_sixty_four_with_a_count_of_the_rest() {
    let evicted: Vec<ChatMessage> = (0..70)
        .map(|index| {
            coding_call(
                &format!("c{index}"),
                "read",
                serde_json::json!({"path": format!("src/file{index}.rs")}),
            )
        })
        .collect();
    let digest = build_digest(&evicted);
    let lines = digest
        .content
        .lines()
        .filter(|line| line.starts_with("read src/file"))
        .count();
    assert_eq!(lines, 64, "{}", digest.content);
    assert!(
        digest.content.contains("… and 6 more tool calls"),
        "{}",
        digest.content
    );
}

/// A path longer than 256 bytes is truncated at the byte bound.
#[test]
fn an_overlong_path_is_truncated_to_256_bytes() {
    let path = "a".repeat(300);
    let evicted = vec![coding_call("r1", "read", serde_json::json!({"path": path}))];
    let digest = build_digest(&evicted);
    assert!(digest.content.contains(&"a".repeat(256)));
    assert!(!digest.content.contains(&"a".repeat(257)));
}

/// A path carrying a credential-shaped value records no line at all.
#[test]
fn a_credential_shaped_path_records_no_line() {
    let evicted = vec![coding_call(
        "r1",
        "read",
        serde_json::json!({"path": "src/api_key=sk-live-CANARY_PATH.rs"}),
    )];
    let digest = build_digest(&evicted);
    assert!(!digest.content.contains("CANARY_PATH"));
    assert!(!digest.content.contains("api_key"));
}

/// `follow_up_child` stays excluded from the digest.
#[test]
fn follow_up_child_remains_excluded_from_the_digest() {
    let evicted = vec![coding_call(
        "f1",
        "follow_up_child",
        serde_json::json!({"path": "src/lib.rs", "agent_id": "CANARY_AGENT"}),
    )];
    let digest = build_digest(&evicted);
    assert!(!digest.content.contains("src/lib.rs"));
    assert!(!digest.content.contains("CANARY_AGENT"));
}

// Coding tools are not SQL findings, even when a refused/forged call carries
// extra SQL-shaped arguments. Existing SQL tests do not cover new coding names.
#[test]
fn deletion_and_followup_raw_bodies_never_enter_the_sql_compaction_digest() {
    for name in ["delete", "follow_up_child"] {
        let mut message =
            assistant_with_call("forged", "SOURCE_BODY_SENTINEL ordinary source text");
        message.tool_calls[0].name = name.into();
        let result = tool_result(
            "forged",
            r#"{"columns":["TOOL_BODY_SENTINEL"],"row_count":1}"#,
        );
        let digest = build_digest(&[message, result]);
        assert!(
            !digest.content.contains("SOURCE_BODY_SENTINEL"),
            "{name} arguments leaked"
        );
        assert!(
            !digest.content.contains("TOOL_BODY_SENTINEL"),
            "{name} result leaked"
        );
        assert!(
            !digest.content.contains("statement:"),
            "coding bodies are not SQL facts"
        );
    }
}
