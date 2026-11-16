/// `/history` and `/sessions` are one command under two names: both map to
/// `SessionAction::History` (saved sessions on disk). This is intentional
/// aliasing, not a bug — see `slash::tests::history_help_names_the_alias`
/// for the matching requirement that the help makes the aliasing explicit.
#[test]
fn history_and_sessions_map_to_the_same_action() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    let history = state.apply(SlashCommand::History, &[]);
    let sessions = state.apply(SlashCommand::Sessions, &[]);
    assert_eq!(history, SessionAction::History);
    assert_eq!(sessions, SessionAction::History);
}

/// `/usage` returns a `SessionAction::Message` carrying the session usage
/// breakdown. The message is handled by the existing `Message` arm in both
/// the TUI dispatch and the headless `emit_action`, so no dispatch-layer
/// changes were needed.
#[test]
fn usage_returns_breakdown_message() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    state
        .usage
        .record(&saya_agent::TokenUsage::new(100, 50).with_cached_input(Some(80)));
    let action = state.apply(SlashCommand::Usage, &[]);
    let SessionAction::Message(msg) = action else {
        panic!("expected SessionAction::Message, got {action:?}");
    };
    assert!(
        msg.contains("Input tokens: 100"),
        "breakdown must show input total, got:\n{msg}"
    );
    assert!(
        msg.contains("Output tokens: 50"),
        "breakdown must show output total, got:\n{msg}"
    );
    assert!(
        msg.contains("Cache hit rate: 80%"),
        "breakdown must show computed hit rate, got:\n{msg}"
    );
    assert!(
        msg.contains("Σcached / Σinput"),
        "breakdown must state the formula, got:\n{msg}"
    );
}

/// `/usage` on an empty session (no turns with usage) reports nothing
/// rather than a row of zeros.
#[test]
fn usage_on_empty_session_reports_nothing() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    let action = state.apply(SlashCommand::Usage, &[]);
    let SessionAction::Message(msg) = action else {
        panic!("expected SessionAction::Message, got {action:?}");
    };
    assert!(
        msg.contains("No token usage reported yet"),
        "empty session must say no usage was reported, got:\n{msg}"
    );
}

/// `/clear` resets the usage accumulator along with the conversation, so a
/// fresh context does not inherit the prior turns' token accounting.
#[test]
fn clear_resets_usage_accumulator() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    state.usage.record(&saya_agent::TokenUsage::new(100, 50));
    assert_eq!(state.usage.answering.turns, 1);
    state.apply(SlashCommand::Clear, &[]);
    assert_eq!(state.usage.answering.turns, 0);
    assert_eq!(state.usage.answering.input_tokens, 0);
}
