/// O1 property 7: the OpenAI body carries the configured `max_output_tokens`
/// as `max_completion_tokens` (`max_tokens` is deprecated and rejected by
/// newer reasoning models).
#[tokio::test]
async fn openai_body_carries_max_completion_tokens() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n"],
    }]);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_max_output_tokens(1234),
        Some("k"),
    )
    .unwrap();
    provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    let sent = requests.lock().unwrap()[0].clone();
    assert!(
        sent.contains("\"max_completion_tokens\":1234"),
        "openai body must carry max_completion_tokens: {sent}"
    );
}
