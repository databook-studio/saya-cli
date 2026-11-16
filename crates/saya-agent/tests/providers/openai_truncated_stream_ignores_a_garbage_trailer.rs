#[tokio::test]
async fn openai_truncated_stream_ignores_a_garbage_trailer() {
    use saya_agent::{ProviderError, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"data: {"choices":[{"delta":{"content":"partial"}}]}

data: {"choices":[{"delta":{},"finish_reason":"length"}]}

"#,
            r#"data: {"choices":[],"usage":{"prompt_tokens":9,"completion_tokens":11,"total_tokens":20}}

data: {not json

"#,
        ],
    }]);
    let mut stream = openai(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_results(&mut stream).await;
    handle.join().unwrap();
    let usage_at = events
        .iter()
        .position(|item| item == &Ok(ProviderEvent::Usage(TokenUsage::new(9, 11))))
        .expect("the trailing usage frame is charged");
    let last = events.last().expect("a terminal item").as_ref();
    let ProviderError::OutputTruncated { partial_text, .. } = last.unwrap_err() else {
        panic!("expected the typed truncation, got: {events:?}");
    };
    assert_eq!(partial_text, "partial");
    assert!(
        usage_at < events.len() - 1,
        "usage precedes the error: {events:?}"
    );
}
