#[test]
fn test_provider_none_lists_available() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    let action = state.apply(SlashCommand::Provider(None), &[]);
    if let SessionAction::Message(msg) = action {
        assert!(msg.contains("available:"));
        assert!(msg.contains("anthropic"));
    } else {
        panic!("Expected SessionAction::Message");
    }
}

#[test]
fn test_model_none_lists_known_models_for_ollama() {
    let mut state = SessionState::new("test", None, "qwen2.5-coder:14b");
    state.provider = "ollama".into();
    let action = state.apply(SlashCommand::Model(None), &[]);
    if let SessionAction::Message(msg) = action {
        assert!(msg.contains("qwen2.5-coder:14b"));
    } else {
        panic!("Expected SessionAction::Message");
    }
}

#[test]
fn test_provider_some_sets_provider_without_available() {
    let mut state = SessionState::new("test", None, "gpt-4o");
    let action = state.apply(SlashCommand::Provider(Some("openai".into())), &[]);
    assert_eq!(state.provider, "openai");
    if let SessionAction::Message(msg) = action {
        assert_eq!(msg, "Provider: openai");
        assert!(!msg.contains("available:"));
    } else {
        panic!("Expected SessionAction::Message");
    }
}

/// `/export` carries the parsed request through unchanged — the TUI's
/// export dispatch owns the behaviour; the adapter owns nothing but the
/// mapping.
#[test]
fn export_request_maps_to_the_same_action() {
    use crate::slash::{ExportMode, ExportRequest};
    let mut state = SessionState::new("test", None, "gpt-4o");
    let request = ExportRequest {
        mode: Some(ExportMode::Snapshot),
        overwrite: true,
        path: "out.csv".into(),
    };
    let action = state.apply(SlashCommand::Export(request.clone()), &[]);
    assert_eq!(action, SessionAction::Export(request));
}

/// `/report` carries the parsed request through unchanged — the TUI's
/// report dispatch owns the behaviour; the adapter owns nothing but the
/// mapping.
#[test]
fn report_request_maps_to_the_same_action() {
    use crate::slash::ReportRequest;
    let mut state = SessionState::new("test", None, "gpt-4o");
    let request = ReportRequest {
        path: "out.md".into(),
        rows: Some(20),
        overwrite: true,
    };
    let action = state.apply(SlashCommand::Report(request.clone()), &[]);
    assert_eq!(action, SessionAction::Report(request));
}

/// `/investigation` carries the translated command through unchanged —
/// both loops hand it to the shared `run_investigation` operation; the
/// adapter owns nothing but the mapping.
#[test]
fn investigation_command_maps_to_the_same_action() {
    use crate::cli::InvestigationCommand;
    let mut state = SessionState::new("test", None, "gpt-4o");
    let command = InvestigationCommand::List {
        limit: None,
        offset: None,
    };
    let action = state.apply(SlashCommand::Investigation(command.clone()), &[]);
    assert_eq!(action, SessionAction::Investigation(command));
}
