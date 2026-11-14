use super::{bounded_post_edit_feedback, committed_result_cap_error};
use crate::commands::coding::{KnownWorkspaceMutation, KnownWorkspaceMutationOperation};
use saya_agent::ToolError;
use serde_json::{Value, json};

fn committed() -> KnownWorkspaceMutation {
    KnownWorkspaceMutation::new(
        KnownWorkspaceMutationOperation::Write,
        "src/lib.rs".into(),
        "a".repeat(64),
        17,
    )
    .unwrap()
}

fn result() -> Value {
    json!({"path":"src/lib.rs","digest":"a".repeat(64),"size":17,"bytes_written":17})
}

fn feedback(committed: &KnownWorkspaceMutation, messages: Vec<String>) -> Value {
    json!({
        "status":"complete",
        "path":committed.path().as_str(),
        "document_sha256":committed.sha256().as_str(),
        "version":1,
        "diagnostics":messages.into_iter().map(|message| json!({"message":message})).collect::<Vec<_>>(),
        "edits_applied":false
    })
}

fn combined_result(base: &Value, feedback: Value) -> Value {
    let mut combined = base.clone();
    combined["post_edit_diagnostics"] = feedback;
    combined
}

#[test]
fn complete_feedback_is_returned_when_the_whole_envelope_fits() {
    let committed = committed();
    let base = result();
    let expected = feedback(&committed, vec!["one diagnostic".into()]);
    let actual = bounded_post_edit_feedback(&base, expected.clone(), 4096)
        .expect("the complete result fits");

    assert_eq!(actual, expected);
    assert!(!saya_agent::shape_tool_result(&combined_result(&base, actual), 4096).truncated);
}

#[test]
fn oversized_diagnostics_are_trimmed_without_clipping_the_envelope() {
    let committed = committed();
    let base = result();
    let expected = feedback(&committed, vec!["diagnostic ".repeat(100); 4]);
    let actual = bounded_post_edit_feedback(&base, expected, 1600)
        .expect("the minimum feedback envelope fits");
    let diagnostics = actual["diagnostics"].as_array().unwrap();

    assert!(!diagnostics.is_empty());
    assert!(diagnostics.len() < 4);
    assert_eq!(actual["diagnostics_truncated"], true);
    assert!(!saya_agent::shape_tool_result(&combined_result(&base, actual), 1600).truncated);
}

#[test]
fn redaction_expansion_is_included_when_fitting_diagnostics() {
    let committed = committed();
    let base = result();
    let secret = format!("token={} ", "sensitive-value".repeat(4));
    let expected = feedback(&committed, vec![secret.clone(), "ordinary é ".repeat(200)]);
    let unredacted = combined_result(&base, expected.clone());
    let cap = unredacted.to_string().len();
    let shaped_unredacted = saya_agent::shape_tool_result(&unredacted, cap);
    assert!(shaped_unredacted.redactions > 0);
    assert!(shaped_unredacted.truncated);

    let actual = bounded_post_edit_feedback(&base, expected, cap)
        .expect("the shorter redacted feedback envelope fits");
    assert_eq!(actual["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(actual["diagnostics_truncated"], true);
    let shaped = saya_agent::shape_tool_result(&combined_result(&base, actual), cap);
    assert!(!shaped.truncated);
    assert_eq!(shaped.redactions, 1);
    assert!(shaped.text.contains("token=[redacted]"));
    assert!(!shaped.text.contains(secret.as_str()));
}

#[test]
fn impossible_and_zero_caps_return_effectful_runner_errors() {
    let committed = committed();
    let base = result();
    let expected = feedback(&committed, vec!["diagnostic".repeat(100)]);
    for cap in [0, 64] {
        assert!(bounded_post_edit_feedback(&base, expected.clone(), cap).is_none());
        let error = committed_result_cap_error(&committed, cap);
        assert!(matches!(&error, ToolError::Runner(_)));
        assert!(!error.proves_no_effect());
    }

    let error = committed_result_cap_error(&committed, 1024);
    let ToolError::Runner(message) = error else {
        panic!("committed facts use an effectful runner error");
    };
    assert!(message.contains("src/lib.rs"));
    let digest = "a".repeat(64);
    assert!(message.contains(digest.as_str()));
    assert!(message.contains("17"));
    assert!(!saya_agent::shape_tool_result(&json!({"error":message}), 1024).truncated);
}
