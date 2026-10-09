#[test]
fn session_slash_state_becomes_the_next_prompt_override() {
    let mut state = crate::SessionState::new("session", None, "initial-model");
    state.apply(
        crate::SlashCommand::Provider(Some("openai_compatible".into())),
        &[],
    );
    state.apply(crate::SlashCommand::Model(Some("next-model".into())), &[]);
    state.apply(crate::SlashCommand::Privacy(Some(true)), &[]);
    state.apply(
        crate::SlashCommand::Connect("analytics".into()),
        &["analytics".into()],
    );
    let overrides = state.prompt_overrides();
    assert_eq!(
        overrides.provider,
        Some(saya_config::AiProvider::OpenaiCompatible)
    );
    assert_eq!(overrides.model.as_deref(), Some("next-model"));
    assert_eq!(overrides.allow_data_sharing, Some(true));
    assert_eq!(overrides.profile.as_deref(), Some("analytics"));
}

#[test]
fn changing_provider_clears_the_previous_provider_endpoint_in_both_directions() {
    let ollama = saya_config::ResolvedAi {
        provider: saya_config::AiProvider::Ollama,
        model: "model".into(),
        base_url: Some("http://ollama.invalid".into()),
        api_key: None,
        allow_data_sharing: false,
        temperature: 0.1,
        timeout_seconds: 60,
        idle_timeout_seconds: 90,
        max_output_tokens: 4096,
        max_output_tokens_is_default: true,
        context_byte_budget: 256 * 1024,
        context_window_tokens: None,
        show_thinking: false,
        compaction: saya_config::CompactionMode::Auto,
        retry_delays_ms: vec![250, 500, 1000],
    };
    let to_openai = crate::agent::runtime::PromptOverrides {
        provider: Some(saya_config::AiProvider::OpenaiCompatible),
        ..Default::default()
    };
    assert_eq!(
        crate::agent::runtime::effective_ai(&ollama, &to_openai).base_url,
        None
    );

    let openai = saya_config::ResolvedAi {
        provider: saya_config::AiProvider::OpenaiCompatible,
        base_url: Some("https://cloud.invalid/v1".into()),
        ..ollama
    };
    let to_ollama = crate::agent::runtime::PromptOverrides {
        provider: Some(saya_config::AiProvider::Ollama),
        ..Default::default()
    };
    assert_eq!(
        crate::agent::runtime::effective_ai(&openai, &to_ollama).base_url,
        None
    );
}

#[test]
fn clear_removes_canonical_turns_and_visible_messages() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.record_turn(
        "first prompt",
        "first answer",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
            ..Default::default()
        }],
    );
    assert!(state.provider_history().len() == 2);
    state.apply(crate::SlashCommand::Clear, &[]);
    assert!(state.messages.is_empty());
    assert!(state.turns.is_empty());
    assert!(state.provider_history().is_empty());
}

#[test]
fn canonical_redacted_turns_do_not_duplicate_legacy_messages_or_tool_payloads() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.record_turn(
        "prompt ROW_SENTINEL",
        "answer ROW_SENTINEL",
        true,
        vec![ToolMetadata {
            name: "bounded_sql_query".into(),
            status: "completed".into(),
            ..Default::default()
        }],
    );
    let saved = state.redacted();
    assert!(saved.messages.is_empty());
    assert_eq!(saved.turns.len(), 1);
    assert!(
        !serde_json::to_string(&saved)
            .unwrap()
            .contains("raw tool arguments")
    );
}

#[test]
fn cloud_privacy_omits_only_database_derived_turns() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.provider = "openai".into();
    state.record_turn("safe prompt", "safe answer", false, Vec::new());
    state.record_turn("row prompt", "CLOUD_ROW_SENTINEL", true, Vec::new());
    let history = state.provider_history();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].content, "safe prompt");
    assert!(
        !serde_json::to_string(&history)
            .unwrap()
            .contains("CLOUD_ROW_SENTINEL")
    );
    state.allow_data_sharing = true;
    assert!(
        serde_json::to_string(&state.provider_history())
            .unwrap()
            .contains("CLOUD_ROW_SENTINEL")
    );
}
