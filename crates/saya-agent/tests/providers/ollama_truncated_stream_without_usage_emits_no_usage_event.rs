#[tokio::test]
async fn ollama_truncated_stream_without_usage_emits_no_usage_event() {
    use saya_agent::{ProviderError, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"partial\"},\"done\":false}\n",
            r#"{"done":true,"done_reason":"length"}"#,
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
        2,
        "text then truncation expected, got: {events:?}"
    );
    assert_eq!(events[0], Ok(ProviderEvent::TextDelta("partial".into())));
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(ProviderEvent::Usage(_)))),
        "a truncated response without counts must not report usage, got: {events:?}"
    );
    let error = events[1].as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
}
