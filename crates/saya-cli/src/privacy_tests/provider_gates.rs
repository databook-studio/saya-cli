#[test]
fn anthropic_is_cloud_gated_on_data_sharing() {
    assert!(!crate::agent::runtime::query_data_allowed(
        saya_config::AiProvider::Anthropic,
        false
    ));
    assert!(crate::agent::runtime::query_data_allowed(
        saya_config::AiProvider::Anthropic,
        true
    ));
}

#[test]
fn gemini_is_cloud_gated_on_data_sharing() {
    assert!(!crate::agent::runtime::query_data_allowed(
        saya_config::AiProvider::Gemini,
        false
    ));
    assert!(crate::agent::runtime::query_data_allowed(
        saya_config::AiProvider::Gemini,
        true
    ));
}
