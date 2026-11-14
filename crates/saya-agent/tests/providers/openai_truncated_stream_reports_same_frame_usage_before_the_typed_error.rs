#[tokio::test]
async fn openai_truncated_stream_reports_same_frame_usage_before_the_typed_error() {
    use saya_agent::{ProviderError, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"data: {"choices":[{"delta":{"content":"partial"}}]}

"#,
            r#"data: {"usage":{"prompt_tokens":9,"completion_tokens":11,"total_tokens":20},"choices":[{"delta":{},"finish_reason":"length"}]}

"#,
            r#"data: [DONE]

"#,
        ],
    }]);
    let mut stream = openai(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_results(&mut stream).await;
    handle.join().unwrap();
    let last = events.last().expect("a terminal item");
    let error = last.as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
    let usage_at = events
        .iter()
        .position(|item| matches!(item, Ok(ProviderEvent::Usage(_))));
    assert_eq!(
        usage_at.map(|index| &events[index]),
        Some(&Ok(ProviderEvent::Usage(TokenUsage::new(9, 11)))),
        "usage must surface before the typed error, got: {events:?}"
    );
    assert!(usage_at.unwrap() < events.len() - 1);
}
