use super::*;

// --- the default stream(), for a provider with no streaming endpoint ----

/// A provider whose only path is `complete()` (Gemini is shaped exactly
/// this way) carries every response field through the default stream and
/// collect adapters without changing values or collapsing absent fields.
#[tokio::test]
async fn the_default_stream_preserves_the_complete_response() {
    struct ResponseOnlyProvider {
        response: ChatResponse,
        complete_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl ChatProvider for ResponseOnlyProvider {
        fn name(&self) -> &str {
            "response-only"
        }
        async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.complete_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(self.response.clone())
        }
        // `stream` is deliberately not overridden: the default is under
        // test, reached directly below exactly as the loop reaches it.
    }

    let complete_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let response = ChatResponse {
        message: ChatMessage {
            role: "assistant".into(),
            content: "The result is 42.".into(),
            tool_calls: vec![crate::ToolCall {
                id: "designation".into(),
                name: "designate_answer".into(),
                arguments: serde_json::json!({"sql": "SELECT 42"}),
            }],
            tool_call_id: None,
        },
        reasoning: Some(String::new()),
        usage: Some(TokenUsage {
            input_tokens: 12,
            output_tokens: 9,
            cached_input_tokens: Some(0),
            ..Default::default()
        }),
    };
    let with_response = ResponseOnlyProvider {
        response: response.clone(),
        complete_calls: complete_calls.clone(),
    };
    let mut stream = with_response
        .stream(
            ChatRequest {
                model: "m".into(),
                messages: Vec::new(),
                tools: Vec::new(),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await
        .expect("stream opens");
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.expect("stream event"));
    }
    assert_eq!(
        events,
        vec![
            ProviderEvent::Usage(response.usage.expect("reported usage")),
            ProviderEvent::TextDelta("The result is 42.".into()),
            ProviderEvent::ReasoningDelta(String::new()),
            ProviderEvent::ToolCalls(response.message.tool_calls.clone()),
            ProviderEvent::Done,
        ],
        "settled usage leads the full-field stream, with every field once and Done last"
    );

    let collected = with_response
        .collect(ChatRequest {
            model: "m".into(),
            messages: Vec::new(),
            tools: Vec::new(),
            ..Default::default()
        })
        .await
        .expect("the complete response is collectable");
    assert_eq!(collected.message.content, response.message.content);
    assert_eq!(collected.message.tool_calls, response.message.tool_calls);
    assert_eq!(collected.reasoning, Some(String::new()));
    assert_eq!(collected.usage, response.usage);
    assert_eq!(complete_calls.load(std::sync::atomic::Ordering::SeqCst), 2);

    // Absence remains distinct from an explicitly reported empty value.
    let silent = ResponseOnlyProvider {
        response: ChatResponse::new(ChatMessage::text("assistant", "hi")),
        complete_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
    };
    let mut stream = silent
        .stream(
            ChatRequest {
                model: "m".into(),
                messages: Vec::new(),
                tools: Vec::new(),
                ..Default::default()
            },
            CancellationToken::new(),
        )
        .await
        .expect("stream opens");
    let mut silent_events = Vec::new();
    while let Some(event) = stream.next().await {
        silent_events.push(event.expect("stream event"));
    }
    assert_eq!(
        silent_events,
        vec![ProviderEvent::TextDelta("hi".into()), ProviderEvent::Done]
    );
    let silent_response = silent
        .collect(ChatRequest::new("m", Vec::new()))
        .await
        .unwrap();
    assert_eq!(silent_response.reasoning, None);
    assert_eq!(silent_response.usage, None);
}

#[tokio::test]
async fn pre_cancelled_default_stream_does_not_call_complete() {
    struct ResponseOnlyProvider(std::sync::Arc<std::sync::atomic::AtomicUsize>);

    #[async_trait]
    impl ChatProvider for ResponseOnlyProvider {
        fn name(&self) -> &str {
            "response-only"
        }
        async fn complete(&self, _request: ChatRequest) -> Result<ChatResponse, ProviderError> {
            self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(ChatResponse::new(ChatMessage::text("assistant", "hi")))
        }
    }

    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let provider = ResponseOnlyProvider(calls.clone());
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    assert!(matches!(
        provider
            .stream(ChatRequest::new("m", Vec::new()), cancellation)
            .await,
        Err(ProviderError::Cancelled)
    ));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
}
