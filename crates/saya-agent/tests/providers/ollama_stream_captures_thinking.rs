/// `message.thinking` is parsed into a
/// `ReasoningDelta` and threaded onto `ChatResponse.reasoning`.
#[tokio::test]
async fn ollama_stream_captures_thinking() {
    use saya_agent::{OllamaProvider, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"\",\"thinking\":\"I considered the schema\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(
        response.reasoning.as_deref(),
        Some("I considered the schema"),
        "message.thinking must reach ChatResponse.reasoning"
    );

    // Observe the ReasoningDelta event directly.
    let (base2, _, handle2) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"thinking\":\"t\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base2))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_reasoning(&mut stream).await;
    handle2.join().unwrap();
    assert!(
        events.contains(&ProviderEvent::ReasoningDelta("t".into())),
        "message.thinking must emit a ReasoningDelta: {events:?}"
    );
}
