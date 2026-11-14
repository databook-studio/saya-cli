#[tokio::test]
async fn every_streaming_provider_applies_the_establishment_deadline() {
    use saya_agent::AnthropicProvider;

    let (base, handle) = silent_server();
    let error = OpenAiCompatibleProvider::new(
        ProviderSettings::new("m", Some(format!("{base}/v1")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("timed out"), "{error}");

    let (base, handle) = silent_server();
    let error = AnthropicProvider::new(
        ProviderSettings::new("m", Some(format!("{base}/v1")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("timed out"), "{error}");

    let (base, handle) = silent_server();
    let error = OllamaProvider::new(
        ProviderSettings::new("m", Some(base))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("timed out"), "{error}");
}
