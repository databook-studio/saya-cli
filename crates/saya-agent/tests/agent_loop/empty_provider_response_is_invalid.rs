#[tokio::test]
async fn empty_provider_response_is_invalid() {
    let provider = MockProvider {
        responses: Mutex::new(vec![ChatResponse::new(ChatMessage::text("assistant", ""))]),
    };
    let error = run_agent(
        &provider,
        &MockTools {
            calls: Arc::new(Mutex::new(Vec::new())),
        },
        request(),
        definitions(),
        AgentLimits::default(),
        &AllowReadOnlyApproval,
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        AgentError::Provider(saya_agent::ProviderError::InvalidResponse)
    ));
}
