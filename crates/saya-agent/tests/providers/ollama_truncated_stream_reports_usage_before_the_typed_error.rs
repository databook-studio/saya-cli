#[tokio::test]
async fn ollama_truncated_stream_reports_usage_before_the_typed_error() {
    use saya_agent::{ProviderError, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"partial\"},\"done\":false}\n",
            r#"{"done":true,"done_reason":"length","prompt_eval_count":9,"eval_count":11}"#,
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_results(&mut stream).await;
    handle.join().unwrap();
    assert_eq!(
        events.len(),
        3,
        "text, usage, truncation expected, got: {events:?}"
    );
    assert_eq!(events[0], Ok(ProviderEvent::TextDelta("partial".into())));
    assert_eq!(
        events[1],
        Ok(ProviderEvent::Usage(TokenUsage::new(9, 11))),
        "usage must surface before the typed truncation error, got: {events:?}"
    );
    let error = events[2].as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
}
