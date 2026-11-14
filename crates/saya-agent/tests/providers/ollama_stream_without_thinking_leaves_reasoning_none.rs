/// a chunk with no `thinking` leaves
/// `ChatResponse.reasoning` `None`.
#[tokio::test]
async fn ollama_stream_without_thinking_leaves_reasoning_none() {
    use saya_agent::OllamaProvider;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(response.reasoning, None);
}
