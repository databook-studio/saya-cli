use super::*;

#[test]
fn an_unseen_statement_has_no_failures() {
    let set = FailedStatements::new();
    assert_eq!(set.failures(&CallKey::Sql("SELECT 1".into())), 0);
}

#[test]
fn recording_a_failure_counts_it() {
    let mut set = FailedStatements::new();
    assert_eq!(set.record_failure(CallKey::Sql("SELECT 1".into())), None);
    assert_eq!(set.failures(&CallKey::Sql("SELECT 1".into())), 1);
}

#[test]
fn a_statement_differing_by_one_character_is_a_different_call() {
    let mut set = FailedStatements::new();
    set.record_failure(CallKey::Sql("SELECT 1".into()));
    // No normalisation: a one-character edit is a different statement.
    assert_eq!(set.failures(&CallKey::Sql("SELECT 2".into())), 0);
}

#[test]
fn repeating_a_failure_advances_the_count_in_place() {
    let mut set = FailedStatements::new();
    let key = CallKey::Sql("SELECT 1".into());
    set.record_failure(key.clone());
    set.record_failure(key.clone());
    assert_eq!(set.failures(&key), 2);
}

/// The note is offered on exactly the third failure of one key: not the first,
/// not the second, and not the fourth.
#[test]
fn the_note_is_offered_once_on_the_third_failure() {
    let mut set = FailedStatements::new();
    let key = CallKey::Sql("SELECT 1".into());
    assert_eq!(set.record_failure(key.clone()), None, "first failure");
    assert_eq!(set.record_failure(key.clone()), None, "second failure");
    assert_eq!(
        set.record_failure(key.clone()),
        Some(REPEAT_NOTE),
        "the third identical failure carries the note"
    );
    assert_eq!(
        set.record_failure(key.clone()),
        None,
        "the fourth does not repeat it"
    );
    assert_eq!(set.failures(&key), 4);
}

/// A success clears the call's failures, so the count of a later failure
/// starts again — the note is about a run of identical failures, not a tally
/// over the whole run.
#[test]
fn a_success_forgets_the_call() {
    let mut set = FailedStatements::new();
    let key = CallKey::Sql("SELECT 1".into());
    set.record_failure(key.clone());
    set.record_failure(key.clone());
    set.forget(&key);
    assert_eq!(set.failures(&key), 0);
    assert_eq!(set.record_failure(key.clone()), None, "the count restarts");
    assert_eq!(set.failures(&key), 1);
}

/// The note is part of the result the model reads, appended after the error —
/// so the cause still comes first and a tight result cap can only ever cut the
/// guidance, never the failure itself.
#[test]
fn the_note_is_appended_to_the_failed_result() {
    let mut result = serde_json::json!({"error": "syntax error near 'bad'"});
    append_note(&mut result, REPEAT_NOTE);
    let error = result["error"]
        .as_str()
        .expect("the result carries its error");
    assert!(
        error.starts_with("syntax error near 'bad'"),
        "the cause comes first: {error}"
    );
    assert!(
        error.ends_with(REPEAT_NOTE),
        "the note is appended: {error}"
    );

    let shaped = super::super::shape_tool_result(&result, 512);
    assert!(shaped.text.contains(REPEAT_NOTE), "{}", shaped.text);
}

/// A failure whose result carries no `error` string still gets the note, under
/// its own key: the advice is never silently dropped.
#[test]
fn a_result_without_an_error_carries_the_note_under_its_own_key() {
    let mut result = serde_json::json!({"status": "failed"});
    append_note(&mut result, REPEAT_NOTE);
    assert_eq!(result["note"], REPEAT_NOTE);
}

/// A tool-call failure is remembered under its own key — and not under a
/// statement key that spells alike — so one memory holds both paths
/// without letting them collide.
#[test]
fn a_tool_call_failure_is_remembered_under_its_own_key() {
    let mut set = FailedStatements::new();
    let key = CallKey::ToolCall("run_program".into(), r#"{"command":"ls"}"#.into());
    set.record_failure(key.clone());
    assert_eq!(set.failures(&key), 1);
    assert_eq!(set.failures(&CallKey::Sql(r#"{"command":"ls"}"#.into())), 0);
}

/// The set is bounded at `MAX_REMEMBERED_FAILURES`: after recording the
/// cap's worth of distinct statements, the next record evicts the oldest, so
/// its count is forgotten. The cap is the documented bound on memory for a
/// long, degenerate run.
#[test]
fn the_remembered_failure_set_is_bounded_at_its_documented_cap() {
    let mut set = FailedStatements::new();
    for index in 0..MAX_REMEMBERED_FAILURES {
        set.record_failure(CallKey::Sql(format!("SELECT {index}")));
    }
    assert_eq!(
        set.failures(&CallKey::Sql("SELECT 0".into())),
        1,
        "before overflow the oldest entry is still remembered"
    );
    // One more evicts the oldest ("SELECT 0").
    set.record_failure(CallKey::Sql("SELECT overflow".into()));
    assert_eq!(
        set.failures(&CallKey::Sql("SELECT 0".into())),
        0,
        "the oldest entry is evicted once the cap is exceeded"
    );
    assert_eq!(
        set.failures(&CallKey::Sql("SELECT overflow".into())),
        1,
        "the newest entry is remembered after the eviction"
    );
    // A recent entry that was not evicted is still remembered.
    let recent = format!("SELECT {}", MAX_REMEMBERED_FAILURES - 1);
    assert_eq!(
        set.failures(&CallKey::Sql(recent)),
        1,
        "a recent entry survives the eviction"
    );
}

#[test]
fn sql_of_extracts_a_string_sql_argument() {
    let call = ToolCall {
        id: "c1".into(),
        name: "bounded_sql_query".into(),
        arguments: serde_json::json!({"sql": "SELECT 1"}),
    };
    assert_eq!(sql_of(&call), Some("SELECT 1"));
}

#[test]
fn sql_of_is_none_when_there_is_no_sql_argument() {
    let call = ToolCall {
        id: "c1".into(),
        name: "schema_discovery".into(),
        arguments: serde_json::json!({}),
    };
    assert!(sql_of(&call).is_none());
}

#[test]
fn sql_of_is_none_for_a_non_string_sql_argument() {
    let call = ToolCall {
        id: "c1".into(),
        name: "bounded_sql_query".into(),
        arguments: serde_json::json!({"sql": 42}),
    };
    assert!(sql_of(&call).is_none());
}
