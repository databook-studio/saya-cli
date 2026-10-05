use saya_agent::{
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, GeminiProvider, LocalStateEffect,
    ProviderError, ProviderSettings, ToolDefinition, ToolEffect,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

struct Reply {
    status: u16,
    chunks: Vec<&'static str>,
}

fn server(replies: Vec<Reply>) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let captured = Arc::new(Mutex::new(Vec::new()));
    let copy = captured.clone();
    let handle = thread::spawn(move || {
        for reply in replies {
            let (mut stream, _) = listener.accept().unwrap();
            copy.lock().unwrap().push(read_request(&mut stream));
            let body = reply.chunks.concat();
            write!(
                stream,
                "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n",
                reply.status,
                body.len()
            )
            .unwrap();
            for chunk in reply.chunks {
                stream.write_all(chunk.as_bytes()).unwrap();
                stream.flush().unwrap();
                thread::sleep(Duration::from_millis(2));
            }
        }
    });
    (base, captured, handle)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let count = stream.read(&mut buffer).unwrap();
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            header_end = end + 4;
            break;
        }
    }
    let header = String::from_utf8_lossy(&bytes[..header_end]);
    let length = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(0);
    while bytes.len() < header_end + length {
        let count = stream.read(&mut buffer).unwrap();
        bytes.extend_from_slice(&buffer[..count]);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn request() -> ChatRequest {
    ChatRequest::new(
        "gemini-x",
        vec![
            ChatMessage::text("system", "system prompt"),
            ChatMessage::text("user", "hello"),
        ],
    )
    .with_tools(vec![ToolDefinition {
        name: "schema_discovery".into(),
        description: "schema".into(),
        read_only: true,
        parameters: serde_json::json!({"type":"object"}),
        effect: ToolEffect {
            database_data: false,
            external_side_effect: false,
            requires_approval: false,
            local_state: LocalStateEffect::None,
        },
        concurrency: saya_agent::ToolConcurrency::Serial,
        completion: None,
    }])
}

fn gemini(base: String) -> GeminiProvider {
    GeminiProvider::new(
        ProviderSettings::new("gemini-x", Some(format!("{base}/v1beta")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("key-sentinel"),
    )
    .unwrap()
}

#[tokio::test]
async fn test_gemini_text_completion() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![r#"{"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]}"#],
    }]);
    let response = gemini(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "Hello");
    let sent = &requests.lock().unwrap()[0];
    assert!(sent.contains("x-goog-api-key: key-sentinel"));
    assert!(sent.contains("systemInstruction"));
    assert!(sent.contains("functionDeclarations"));
    assert!(sent.contains("generationConfig"));
}

#[tokio::test]
async fn test_gemini_tool_call() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"schema_discovery","args":{"connection":"warehouse"}}}]}}]}"#,
        ],
    }]);
    let response = gemini(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.tool_calls[0].name, "schema_discovery");
    assert_eq!(
        response.message.tool_calls[0].arguments["connection"],
        "warehouse"
    );
}

/// O1 property 3: Gemini `finishReason: "MAX_TOKENS"` is detected — today it
/// passes silently and looks complete. The error is typed and carries the
/// partial text.
#[tokio::test]
async fn gemini_max_tokens_finish_reason_is_a_typed_truncation_error() {
    use saya_agent::ProviderError;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]}"#,
        ],
    }]);
    let error = gemini(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "gemini finishReason MAX_TOKENS must be typed truncation, got: {error:?}"
    );
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        unreachable!("matched above");
    };
    assert_eq!(partial_text, "partial");
}

/// O1 non-truncation pin: a normal Gemini response (`finishReason: "STOP"`)
/// still completes with its content.
#[tokio::test]
async fn gemini_stop_finish_reason_still_completes_with_content() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#,
        ],
    }]);
    let response = gemini(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
}

#[tokio::test]
async fn gemini_establishment_timeout_is_not_a_client_total_timeout() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(socket);
    });
    let provider = GeminiProvider::new(
        ProviderSettings::new("gemini-x", Some(format!("{base}/v1beta")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
        Some("key-sentinel"),
    )
    .unwrap();

    let error = provider.complete(request()).await.unwrap_err();
    assert!(
        matches!(&error, ProviderError::Request(message) if message.contains("timed out")),
        "unexpected establishment error: {error:?}"
    );
    server.await.unwrap();
}

#[tokio::test]
async fn gemini_stream_cancellation_interrupts_establishment() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(socket);
    });
    let provider = GeminiProvider::new(
        ProviderSettings::new("gemini-x", Some(format!("{base}/v1beta")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_secs(5)),
        Some("key-sentinel"),
    )
    .unwrap();
    let cancellation = CancellationToken::new();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move { provider.stream(request(), task_cancellation).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    cancellation.cancel();

    let result = tokio::time::timeout(Duration::from_millis(100), task)
        .await
        .expect("cancellation must finish promptly")
        .unwrap();
    let error = match result {
        Ok(_) => panic!("cancellation must interrupt the Gemini request"),
        Err(error) => error,
    };
    assert_eq!(error, ProviderError::Cancelled);
    server.await.unwrap();
}

#[tokio::test]
async fn gemini_truncated_stream_reports_usage_before_the_typed_error() {
    use futures_util::StreamExt;
    use saya_agent::{ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}],"usageMetadata":{"promptTokenCount":9,"candidatesTokenCount":11}}"#,
        ],
    }]);
    let mut stream = gemini(base)
        .stream(request(), CancellationToken::new())
        .await
        .expect("a truncated response is a stream that ends in the typed error");
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        let terminal = item.is_err();
        items.push(item);
        if terminal {
            break;
        }
    }
    handle.join().unwrap();
    assert_eq!(items.len(), 2, "usage then error, got: {items:?}");
    assert_eq!(items[0], Ok(ProviderEvent::Usage(TokenUsage::new(9, 11))));
    let error = items[1].as_ref().unwrap_err();
    let ProviderError::OutputTruncated { partial_text, .. } = error else {
        panic!("expected typed truncation, got: {error:?}");
    };
    assert_eq!(partial_text, "partial");
}

#[tokio::test]
async fn gemini_truncated_stream_without_usage_metadata_emits_no_usage_event() {
    use futures_util::StreamExt;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            r#"{"candidates":[{"content":{"parts":[{"text":"partial"}]},"finishReason":"MAX_TOKENS"}]}"#,
        ],
    }]);
    let mut stream = gemini(base)
        .stream(request(), CancellationToken::new())
        .await
        .expect("a truncated response is a stream that ends in the typed error");
    let mut items = Vec::new();
    while let Some(item) = stream.next().await {
        items.push(item);
    }
    handle.join().unwrap();
    assert_eq!(items.len(), 1, "only the typed error, got: {items:?}");
    assert!(matches!(
        items[0].as_ref().unwrap_err(),
        ProviderError::OutputTruncated { .. }
    ));
}
