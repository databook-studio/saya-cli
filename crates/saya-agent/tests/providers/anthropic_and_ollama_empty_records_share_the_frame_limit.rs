#[tokio::test]
async fn anthropic_and_ollama_empty_records_share_the_frame_limit() {
    const FRAME_LIMIT: usize = 65_536;
    for provider_kind in ["anthropic", "ollama"] {
        let body = match provider_kind {
            "anthropic" => b"\n\n".repeat(FRAME_LIMIT + 1),
            "ollama" => vec![b'\n'; FRAME_LIMIT + 1],
            _ => unreachable!("the table lists supported framed parsers"),
        };
        let (base, handle) = byte_server(vec![body]);
        let provider: Box<dyn ChatProvider> = match provider_kind {
            "anthropic" => Box::new(
                saya_agent::AnthropicProvider::new(
                    ProviderSettings::new("test-model", Some(base)).with_retry_delays(Vec::new()),
                    Some("secret-sentinel"),
                )
                .unwrap(),
            ),
            "ollama" => Box::new(
                OllamaProvider::new(
                    ProviderSettings::new("test-model", Some(base)).with_retry_delays(Vec::new()),
                )
                .unwrap(),
            ),
            _ => unreachable!("the table lists supported framed parsers"),
        };
        let mut stream = provider
            .stream(request(), CancellationToken::new())
            .await
            .unwrap();
        assert!(
            matches!(
                stream.next().await.unwrap(),
                Err(ProviderError::StreamItemLimit { .. })
            ),
            "{provider_kind} did not stop at the shared item limit"
        );
        handle.join().unwrap();
    }
}
