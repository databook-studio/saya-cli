#[test]
fn remote_ollama_is_cloud_gated_but_loopback_and_explicit_sharing_work() {
    assert!(!crate::agent::runtime::query_data_allowed_for_endpoint(
        saya_config::AiProvider::Ollama,
        Some("https://ollama.example.invalid"),
        false,
    ));
    assert!(crate::agent::runtime::query_data_allowed_for_endpoint(
        saya_config::AiProvider::Ollama,
        Some("http://127.0.0.1:11434"),
        false,
    ));
    assert!(crate::agent::runtime::query_data_allowed_for_endpoint(
        saya_config::AiProvider::Ollama,
        Some("https://ollama.example.invalid"),
        true,
    ));
}

#[test]
fn ambiguous_ollama_endpoints_fail_closed() {
    for endpoint in [
        "not a url",
        "http://[::1",
        "http://127.0.0.1:11434/?redirect=https://ollama.example.invalid",
    ] {
        assert!(
            !crate::agent::runtime::query_data_allowed_for_endpoint(
                saya_config::AiProvider::Ollama,
                Some(endpoint),
                false,
            ),
            "ambiguous endpoint must not admit database data: {endpoint}"
        );
    }
}

#[test]
fn resumed_ollama_session_does_not_restore_sensitive_history_for_remote_endpoint() {
    let mut state = crate::SessionState::new("session", None, "model");
    state.provider = "ollama".into();
    state.provider_endpoint = Some("https://ollama.example.invalid".into());
    state.record_turn("row prompt", "REMOTE_OLLAMA_ROW", true, Vec::new());

    let restored: crate::SessionState =
        serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
    assert!(
        !serde_json::to_string(&restored.provider_history())
            .unwrap()
            .contains("REMOTE_OLLAMA_ROW")
    );
}

#[test]
fn tui_picker_resume_binds_a_legacy_session_to_the_live_remote_endpoint() {
    // A legacy record: no endpoint carried, unbound, sharing off, with a
    // database-derived turn. Unbound it reads as the provider's own
    // localhost default, so the history is present.
    let loaded = crate::SessionState::new("legacy", None, "model");
    assert!(!loaded.provider_endpoint_bound);
    let mut loaded = loaded;
    loaded.provider = "ollama".into();
    loaded.record_turn("row prompt", "LEGACY_PICKER_SENTINEL", true, Vec::new());
    assert!(
        serde_json::to_string(&loaded.provider_history())
            .unwrap()
            .contains("LEGACY_PICKER_SENTINEL")
    );
    // The TUI picker's resume adopts the loaded record and binds the live
    // runtime endpoint; under a remote runtime the sentinel must not enter
    // history without consent.
    let mut state = crate::SessionState::new("current", None, "model");
    crate::interactive::session::lifecycle::resume::adopt_picker_resumed(
        &mut state,
        loaded,
        Some("https://ollama.example.invalid"),
    );
    assert!(state.provider_endpoint_bound);
    assert!(
        !serde_json::to_string(&state.provider_history())
            .unwrap()
            .contains("LEGACY_PICKER_SENTINEL")
    );
}
