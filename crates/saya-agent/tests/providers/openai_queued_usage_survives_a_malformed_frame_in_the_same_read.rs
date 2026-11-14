#[tokio::test]
async fn openai_queued_usage_survives_a_malformed_frame_in_the_same_read() {
    use saya_agent::{ProviderError, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
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
    assert_eq!(
        events.first(),
        Some(&Ok(ProviderEvent::Usage(TokenUsage::new(9, 11)))),
        "queued usage is delivered before the protocol error: {events:?}"
    );
    assert!(matches!(
        events.last().unwrap().as_ref().unwrap_err(),
        ProviderError::InvalidResponse
    ));
}
