#[tokio::test]
async fn ollama_accepts_terminal_record_without_trailing_newline() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["{\"message\":{\"content\":\"ready\"},\"done\":true}"],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "ready"
    );
    handle.join().unwrap();
}
