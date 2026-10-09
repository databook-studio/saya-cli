#[tokio::test]
async fn ollama_stream_surfaces_eval_counts() {
    use saya_agent::{ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain(&mut stream).await;
    handle.join().unwrap();
    assert!(events.contains(&ProviderEvent::Usage(TokenUsage::new(9, 11))));
}
