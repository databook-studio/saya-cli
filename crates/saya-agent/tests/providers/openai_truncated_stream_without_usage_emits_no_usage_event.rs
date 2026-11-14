#[tokio::test]
async fn openai_truncated_stream_without_usage_emits_no_usage_event() {
    use saya_agent::{ProviderError, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"data: {"choices":[{"delta":{"content":"partial"}}]}

"#,
            r#"data: {"choices":[{"delta":{},"finish_reason":"length"}]}

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
    assert!(
        usage_at.is_none(),
        "no usage figures, no Usage event: {events:?}"
    );
}
