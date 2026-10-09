#[tokio::test]
async fn openai_stream_surfaces_usage_and_requests_it() {
    use saya_agent::{ProviderEvent, TokenUsage};
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":200,\"prompt_tokens_details\":{\"cached_tokens\":90},\"completion_tokens_details\":{\"reasoning_tokens\":170}}}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let response = openai(base.clone()).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(
        response.usage,
        Some(
            TokenUsage::new(100, 200)
                .with_cached_input(Some(90))
                .with_reasoning(Some(170))
        )
    );
    let sent = requests.lock().unwrap()[0].clone();
    assert!(
        sent.contains("\"stream_options\":{\"include_usage\":true}"),
        "must ask the gateway for usage counts: {sent}"
    );
    // Re-run the stream directly to observe the Usage event.
    let (base2, _, handle2) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":100,\"completion_tokens\":200,\"prompt_tokens_details\":{\"cached_tokens\":90},\"completion_tokens_details\":{\"reasoning_tokens\":170}}}\n\ndata: [DONE]\n\n",
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
    let events = drain(&mut stream).await;
    handle2.join().unwrap();
    assert!(
        events.contains(&ProviderEvent::Usage(
            TokenUsage::new(100, 200)
                .with_cached_input(Some(90))
                .with_reasoning(Some(170))
        ))
    );
}
