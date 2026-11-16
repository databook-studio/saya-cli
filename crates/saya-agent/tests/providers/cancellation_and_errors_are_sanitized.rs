#[tokio::test]
async fn cancellation_and_errors_are_sanitized() {
    let (base, _, handle) = server(vec![Reply {
        status: 500,
        chunks: vec!["secret-sentinel"],
    }]);
    let error = openai(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(!error.to_string().contains("secret-sentinel"));
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"slow\"}}]}\n\n"],
    }]);
    let provider = openai(base);
    let token = CancellationToken::new();
    let mut stream = provider.stream(request(), token.clone()).await.unwrap();
    token.cancel();
    assert!(matches!(
        stream.next().await.unwrap(),
        Err(ProviderError::Cancelled)
    ));
    handle.join().unwrap();
}
