/// Every streaming route is bounded by one total per-request deadline, not
/// only by the gap between chunks: a stream that keeps flowing but never ends
/// is cut, and the cut is typed so the loop can continue rather than retry the
/// identical request.
#[tokio::test]
async fn every_streaming_provider_cuts_at_the_request_deadline() {
    use saya_agent::{AnthropicProvider, OllamaProvider};

    let openai_body = "data: {\"choices\":[{\"delta\":{\"content\":\"partial answer\"}}]}\n\n";
    let (base, handle) = keep_open_server(openai_body);
    let error = OpenAiCompatibleProvider::new(
        ProviderSettings::new("m", Some(format!("{base}/v1")))
            .with_retry_delays(Vec::new())
            .with_idle_timeout(Duration::from_secs(5))
            .with_request_timeout(Duration::from_millis(80)),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(
            error,
            ProviderError::StreamCut {
                estimated_tokens,
                ..
            } if estimated_tokens > 0
        ),
        "an OpenAI-compatible stream is cut at the deadline: {error:?}"
    );

    let anthropic_body = "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial answer\"}}\n\n";
    let (base, handle) = keep_open_server(anthropic_body);
    let error = AnthropicProvider::new(
        ProviderSettings::new("m", Some(base))
            .with_retry_delays(Vec::new())
            .with_idle_timeout(Duration::from_secs(5))
            .with_request_timeout(Duration::from_millis(80)),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(
            error,
            ProviderError::StreamCut {
                estimated_tokens,
                ..
            } if estimated_tokens > 0
        ),
        "an Anthropic stream is cut at the deadline: {error:?}"
    );

    let ollama_body = "{\"message\":{\"role\":\"assistant\",\"content\":\"partial answer\"},\"done\":false}\n";
    let (base, handle) = keep_open_server(ollama_body);
    let error = OllamaProvider::new(
        ProviderSettings::new("m", Some(base))
            .with_retry_delays(Vec::new())
            .with_idle_timeout(Duration::from_secs(5))
            .with_request_timeout(Duration::from_millis(80)),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(
            error,
            ProviderError::StreamCut {
                estimated_tokens,
                ..
            } if estimated_tokens > 0
        ),
        "an Ollama stream is cut at the deadline: {error:?}"
    );
}
