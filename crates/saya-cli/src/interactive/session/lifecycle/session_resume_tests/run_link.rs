fn resume_defaults() -> SessionDefaults {
    SessionDefaults {
        provider: "ollama".into(),
        model: "current-model".into(),
        allow_data_sharing: false,
        approval_mode: "ask".into(),
    }
}

/// The link is carried, not consumed: a record that names its run restores it
/// into session state, and the session's next save writes the same job id and
/// the same epoch back.
#[test]
fn the_run_link_survives_the_state_round_trip() {
    let record: RedactedSession = serde_json::from_str(
        r#"{"version":2,"id":"run-linked","run":{"job":"job-051","epoch":7},"messages":[]}"#,
    )
    .unwrap();
    let state = super::state_from_redacted(record, &resume_defaults());
    let rewritten = serde_json::to_value(state.redacted()).unwrap();
    assert_eq!(
        rewritten["run"]["job"], "job-051",
        "the run's job id must ride the state: {rewritten}"
    );
    assert_eq!(
        rewritten["run"]["epoch"], 7,
        "the run's epoch must ride the state: {rewritten}"
    );
}

/// A session with no run behind it resumes exactly as before: nothing sets a
/// link, so the next save carries no `run` key at all.
#[test]
fn a_session_without_a_run_writes_no_run_key() {
    let state = SessionState::new("plain", Some("analytics".into()), "model");
    let rewritten = serde_json::to_value(state.redacted()).unwrap();
    assert!(
        rewritten.get("run").is_none(),
        "a session with no run must not grow the key: {rewritten}"
    );
}
