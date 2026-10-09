use super::*;

#[test]
fn prior_error_is_none_for_an_unseen_statement() {
    let set = FailedStatements::new();
    assert!(set.prior_error(&CallKey::Sql("SELECT 1".into())).is_none());
}

#[test]
fn record_then_prior_error_names_the_failure() {
    let mut set = FailedStatements::new();
    set.record(CallKey::Sql("SELECT 1".into()), "syntax error near '1'");
    assert_eq!(
        set.prior_error(&CallKey::Sql("SELECT 1".into())),
        Some("syntax error near '1'")
    );
}

#[test]
fn a_statement_differing_by_one_character_is_not_a_repeat() {
    let mut set = FailedStatements::new();
    set.record(CallKey::Sql("SELECT 1".into()), "bad");
    // No normalisation: a one-character edit is a different statement.
    assert!(set.prior_error(&CallKey::Sql("SELECT 2".into())).is_none());
}

#[test]
fn recording_the_same_statement_twice_keeps_the_latest_error() {
    let mut set = FailedStatements::new();
    set.record(CallKey::Sql("SELECT 1".into()), "first error");
    set.record(CallKey::Sql("SELECT 1".into()), "second error");
    // The most recent record wins so the refusal names the latest failure.
    assert_eq!(
        set.prior_error(&CallKey::Sql("SELECT 1".into())),
        Some("second error")
    );
}

/// A tool-call failure is remembered under its own key — and not under a
/// statement key that spells alike — so one memory holds both paths
/// without letting them collide.
#[test]
fn a_tool_call_failure_is_remembered_under_its_own_key() {
    let mut set = FailedStatements::new();
    let key = CallKey::ToolCall("run_program".into(), r#"{"command":"ls"}"#.into());
    set.record(key.clone(), "exit 1");
    assert_eq!(set.prior_error(&key), Some("exit 1"));
    assert!(
        set.prior_error(&CallKey::Sql(r#"{"command":"ls"}"#.into()))
            .is_none()
    );
}

/// The set is bounded at `MAX_REMEMBERED_FAILURES`: after recording the
/// cap's worth of distinct statements, the next record evicts the oldest,
/// so a repeat of the evicted statement is no longer refused. The cap is
/// the documented bound on memory for a long, degenerate run.
#[test]
fn the_remembered_failure_set_is_bounded_at_its_documented_cap() {
    let mut set = FailedStatements::new();
    for index in 0..MAX_REMEMBERED_FAILURES {
        set.record(
            CallKey::Sql(format!("SELECT {index}")),
            &format!("err {index}"),
        );
    }
    assert_eq!(
        set.prior_error(&CallKey::Sql("SELECT 0".into())),
        Some("err 0"),
        "before overflow the oldest entry is still remembered"
    );
    // One more evicts the oldest ("SELECT 0").
    set.record(CallKey::Sql("SELECT overflow".into()), "err overflow");
    assert_eq!(
        set.prior_error(&CallKey::Sql("SELECT 0".into())),
        None,
        "the oldest entry is evicted once the cap is exceeded"
    );
    assert_eq!(
        set.prior_error(&CallKey::Sql("SELECT overflow".into())),
        Some("err overflow"),
        "the newest entry is remembered after the eviction"
    );
    // A recent entry that was not evicted is still refused.
    let recent = format!("SELECT {}", MAX_REMEMBERED_FAILURES - 1);
    let recent_err = format!("err {}", MAX_REMEMBERED_FAILURES - 1);
    assert_eq!(
        set.prior_error(&CallKey::Sql(recent)),
        Some(recent_err.as_str()),
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
