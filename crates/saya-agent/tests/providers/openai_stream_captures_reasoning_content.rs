// --- reasoning capture, one test per provider (present → captured) -------

/// `delta.reasoning_content` is parsed into a
/// `ReasoningDelta` event, which `collect()` threads onto `ChatResponse.reasoning`.
/// The chain-of-thought is captured even though no user toggle asked for it
/// (capture is unconditional).
#[tokio::test]
async fn openai_stream_captures_reasoning_content() {
    use saya_agent::ProviderEvent;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"I considered the time column\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6}}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let provider = openai(base);
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(
        response.reasoning.as_deref(),
        Some("I considered the time column"),
        "delta.reasoning_content must reach ChatResponse.reasoning"
    );

    // Re-run the stream directly to observe the ReasoningDelta event.
    let (base2, _, handle2) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: [DONE]\n\n",
        ],
    }]);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base2}/v1"))),
        Some("k"),
    )
    .unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_reasoning(&mut stream).await;
    handle2.join().unwrap();
    assert!(
        events.contains(&ProviderEvent::ReasoningDelta("thinking".into())),
        "ReasoningDelta event must be emitted: {events:?}"
    );
}
