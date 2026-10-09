/// A resumed session carries no stale warn flag: a fresh session starts
/// armed, and an old session file (no such key) deserializes armed too,
/// so the next turn's usage re-derives the flag from a live report.
#[test]
fn the_context_warn_flag_starts_armed_and_stays_out_of_persisted_sessions() {
    let fresh = SessionState::new("s1", None, "m");
    assert!(
        !fresh.context_warned,
        "a fresh session starts with the warning armed (unfired)"
    );
    let json = serde_json::to_string(&fresh).expect("serializes");
    assert!(
        !json.contains("context_warned"),
        "the flag must stay out of persisted sessions: {json}"
    );
    // An old session file predates the flag: it deserializes armed.
    let old = r#"{"id":"s1","profile":null,"included_profiles":[],"provider":"ollama","model":"m","allow_data_sharing":false,"approval_mode":"ask","agent_mode":"build","workspace_root":null,"messages":[],"turns":[]}"#;
    let loaded: SessionState = serde_json::from_str(old).expect("old form deserializes");
    assert!(
        !loaded.context_warned,
        "an old session file resumes with the warning armed (unfired)"
    );
}

/// `/clear` re-arms the context warning along with the conversation: the
/// working memory is fresh, so the next crossing must warn again.
#[test]
fn clear_re_arms_the_context_warning() {
    let mut state = SessionState::new("s1", None, "m");
    state.context_warned = true;
    state.apply(crate::SlashCommand::Clear, &[]);
    assert!(
        !state.context_warned,
        "/clear must re-arm the warning, not leave it fired"
    );
}
