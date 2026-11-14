#[tokio::test]
async fn openai_stream_that_ends_after_partial_text_is_stream_ended() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n"],
    }]);
    let mut stream = openai(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_results(&mut stream).await;
    handle.join().unwrap();
    assert_eq!(
        events.last(),
        Some(&Err(ProviderError::StreamEnded)),
        "an end before [DONE] is the typed cause, not a malformed response: {events:?}"
    );
    assert!(
        events.contains(&Ok(ProviderEvent::TextDelta("partial".into()))),
        "text already streamed is delivered before the error: {events:?}"
    );
}

#[tokio::test]
async fn openai_stream_that_ends_inside_a_tool_call_emits_no_tool_calls_before_stream_ended() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-7\",\"function\":{\"name\":\"schema_discovery\",\"arguments\":\"{\\\"schema\\\":\"}}]}}]}\n\n",
        ],
    }]);
    let mut stream = openai(base)
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_results(&mut stream).await;
    handle.join().unwrap();
    assert_eq!(
        events.last(),
        Some(&Err(ProviderError::StreamEnded)),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Ok(ProviderEvent::ToolCalls(_)))),
        "half-assembled arguments are never handed to the loop: {events:?}"
    );
}
