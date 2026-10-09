/// Both public Ollama call modes send the configured generation reservation;
/// a local server observes the actual JSON body rather than a serializer-only
/// representation.
#[tokio::test]
async fn ollama_complete_and_stream_send_configured_num_predict() {
    let (base, requests, handle) = server(vec![
        Reply {
            status: 200,
            chunks: vec!["{\"message\":{\"content\":\"complete\"},\"done\":true}\n"],
        },
        Reply {
            status: 200,
            chunks: vec!["{\"message\":{\"content\":\"stream\"},\"done\":true}\n"],
        },
    ]);
    let provider = OllamaProvider::new(
        ProviderSettings::new("test", Some(base))
            .with_temperature(0.25)
            .with_max_output_tokens(1234),
    )
    .unwrap();
    let mut request = request();
    request.messages.push(ChatMessage {
        role: "assistant".into(),
        content: String::new(),
        tool_calls: vec![ToolCall {
            id: "call-history".into(),
            name: "schema_discovery".into(),
            arguments: serde_json::json!({"schema": "public"}),
        }],
        tool_call_id: None,
    });

    assert_eq!(
        provider
            .complete(request.clone())
            .await
            .unwrap()
            .message
            .content,
        "complete"
    );
    let mut stream = provider
        .stream(request, CancellationToken::new())
        .await
        .unwrap();
    assert!(
        drain(&mut stream)
            .await
            .contains(&saya_agent::ProviderEvent::Done)
    );
    handle.join().unwrap();

    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2, "one complete and one stream request");
    for request in captured.iter() {
        let body: serde_json::Value = serde_json::from_str(
            request
                .split_once("\r\n\r\n")
                .expect("HTTP request has a body")
                .1,
        )
        .expect("Ollama request body is JSON");
        assert_eq!(body["options"]["num_predict"], 1234);
        assert_eq!(body["options"]["temperature"], 0.25);
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["name"],
            "schema_discovery"
        );
    }
}
