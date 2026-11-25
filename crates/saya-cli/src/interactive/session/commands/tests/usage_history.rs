use crate::slash::UsageRequest;

/// `/history` and `/sessions` are one command under two names: both map to
/// `SessionAction::History` (saved sessions on disk), and only the flag — the
/// one-release alias's note — tells the spellings apart. This is intentional
/// aliasing, not a bug.
#[test]
fn history_and_sessions_map_to_the_same_action() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    let history = state.apply(SlashCommand::Sessions(SessionsRequest::history()), &[]);
    let sessions = state.apply(SlashCommand::Sessions(SessionsRequest::default()), &[]);
    assert_eq!(history, SessionAction::History(true));
    assert_eq!(sessions, SessionAction::History(false));
}

/// `/usage` carries its request to the loop, which holds the runtime's model
/// window and the session's ceiling: bare reads, and `--limit <usd>` also
/// states the ceiling for this session.
#[test]
fn usage_carries_the_panel_request_and_its_ceiling() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    assert_eq!(
        state.apply(SlashCommand::Usage(UsageRequest::show()), &[]),
        SessionAction::Usage(UsageRequest::show())
    );
    assert_eq!(
        state.apply(SlashCommand::Usage(UsageRequest::limit(2_500_000)), &[]),
        SessionAction::Usage(UsageRequest::limit(2_500_000))
    );
}

/// `/usage` on an empty session says there were no requests yet, rather than
/// printing a row of zeros.
#[test]
fn usage_on_empty_session_says_no_request_yet() {
    let state = SessionState::new("test", None, "gpt-4o");
    let panel = state.usage.panel(None, None);
    assert!(
        panel.contains("Session usage — no requests yet"),
        "empty session must say no request was made, got:\n{panel}"
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
