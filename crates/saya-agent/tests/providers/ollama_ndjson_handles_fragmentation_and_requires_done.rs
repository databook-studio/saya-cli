#[tokio::test]
async fn ollama_ndjson_handles_fragmentation_and_requires_done() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"re",
            "ady\"},\"done\":false}\n{\"done\":true}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ready");
    assert!(requests.lock().unwrap()[0].contains("\"stream\":true"));
}
