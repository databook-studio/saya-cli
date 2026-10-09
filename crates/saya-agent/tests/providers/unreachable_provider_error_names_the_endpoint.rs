#[tokio::test]
async fn unreachable_provider_error_names_the_endpoint() {
    use saya_agent::OllamaProvider;
    // Port 1 is reserved and refuses connections: the error must say where
    // we tried to go instead of a bare "network request failed".
    let provider = OllamaProvider::new(ProviderSettings::new(
        "m",
        Some("http://127.0.0.1:1".into()),
    ))
    .unwrap();
    let error = provider.complete(request()).await.unwrap_err().to_string();
    assert!(error.contains("127.0.0.1:1"), "{error}");
    assert!(error.contains("base_url"), "{error}");
}
