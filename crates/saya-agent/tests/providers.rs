use futures_util::StreamExt;
use saya_agent::{
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, LocalStateEffect, OllamaProvider,
    OpenAiCompatibleProvider, ProviderError, ProviderSettings, ToolDefinition, ToolEffect,
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
            write!(stream, "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n", reply.status, body.len()).unwrap();
            for chunk in reply.chunks {
                stream.write_all(chunk.as_bytes()).unwrap();
                stream.flush().unwrap();
                thread::sleep(Duration::from_millis(2));
            }
        }
    });
    (base, captured, handle)
}

fn byte_server(chunks: Vec<Vec<u8>>) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        // The client under test is *expected* to hang up mid-body once the
        // stream cap trips, so writes here race the close: they either fail
        // with EPIPE or block on a socket buffer nobody is draining. The
        // timeout bounds the blocking case and the errors are ignored, because
        // a truncated write is the behaviour being exercised, not a fault.
        // Without both, `handle.join()` can wait forever — this test hung. The
        // timeout is well under the 5s the test itself asserts, so a blocked
        // write cannot push the run past its own deadline.
        let _ = stream.set_write_timeout(Some(Duration::from_millis(250)));
        let length: usize = chunks.iter().map(Vec::len).sum();
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        );
        for chunk in chunks {
            if stream.write_all(&chunk).is_err() {
                break;
            }
            let _ = stream.flush();
        }
    });
    (base, handle)
}

fn keep_open_server(body: &'static str) -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let _ = read_request(&mut stream);
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: keep-alive\r\n\r\n{}", body.len() + 1, body).unwrap();
        stream.flush().unwrap();
        thread::sleep(Duration::from_millis(200));
    });
    (base, handle)
}

fn silent_server() -> (String, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        thread::sleep(Duration::from_millis(150));
        drop(stream);
    });
    (base, handle)
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
    ChatRequest::new("test-model", vec![ChatMessage::text("user", "hello")]).with_tools(vec![
        ToolDefinition {
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
            completion: None,
        },
    ])
}
fn openai(base: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap()
}

/// Cross-host 307 capture rig shared by the three remote-provider redirect
/// tests. The origin answers 307 to `target_url`; the target records every
/// request that reaches it and answers `terminal` (a per-provider
/// terminal payload) so a following client terminates instead of erroring.
struct RedirectCapture {
    origin_base: String,
    reached: Arc<Mutex<Vec<String>>>,
    origin_handle: thread::JoinHandle<()>,
    target_handle: thread::JoinHandle<()>,
}

fn redirect_capture(target_url: &str, terminal: &'static str) -> RedirectCapture {
    // Target host: captures any request that reaches it, then answers the
    // provider's terminal record.
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_base = format!("http://{}", target.local_addr().unwrap());
    let redirect_to = format!("{target_base}{target_url}");
    let reached = Arc::new(Mutex::new(Vec::new()));
    let reached_copy = reached.clone();
    let target_handle = thread::spawn(move || {
        // Nonblocking accept with a bounded deadline: when the client
        // refuses the redirect (the secure behaviour) nothing ever
        // connects, and the capture must end rather than hang the suite.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match target.accept() {
                Ok((mut stream, _)) => {
                    target.set_nonblocking(false).ok();
                    reached_copy.lock().unwrap().push(read_request(&mut stream));
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{terminal}",
                        terminal.len()
                    )
                    .unwrap();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return,
            }
        }
    });
    // Origin host: answers 307 to the target's URL.
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_base = format!("http://{}", origin.local_addr().unwrap());
    let origin_handle = thread::spawn(move || {
        let (mut stream, _) = origin.accept().unwrap();
        let _ = read_request(&mut stream);
        let head = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {redirect_to}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    RedirectCapture {
        origin_base,
        reached,
        origin_handle,
        target_handle,
    }
}

fn await_capture(capture: RedirectCapture) -> (String, Arc<Mutex<Vec<String>>>) {
    capture.origin_handle.join().unwrap();
    capture.target_handle.join().unwrap();
    (capture.origin_base, capture.reached)
}

#[tokio::test]
async fn openai_sse_handles_fragmented_text_and_done() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"Hel",
            "lo\"}}]}\n\n: harmless\n\ndata: [DONE]\n\n",
        ],
    }]);
    let response = openai(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "Hello");
    let sent = &requests.lock().unwrap()[0];
    assert!(sent.contains("\"stream\":true"));
    assert!(sent.contains("Bearer secret-sentinel"));
}

#[tokio::test]
async fn openai_sse_preserves_utf8_and_queued_delta_before_done() {
    let body = b"data: {\"choices\":[{\"delta\":{\"content\":\"planet \xf0\x9f\x8c\x8d\"}}]}\n\ndata: [DONE]\n\n";
    let split = body.iter().position(|byte| *byte == 0xf0).unwrap() + 2;
    let (base, handle) = byte_server(vec![body[..split].to_vec(), body[split..].to_vec()]);
    assert_eq!(
        openai(base)
            .complete(request())
            .await
            .unwrap()
            .message
            .content,
        "planet 🌍"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn terminal_sentinels_allow_only_trailing_ascii_whitespace() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n \r\n\t",
        ],
    }]);
    assert_eq!(
        openai(base)
            .complete(request())
            .await
            .unwrap()
            .message
            .content,
        "ok"
    );
    handle.join().unwrap();
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\nnot-whitespace",
        ],
    }]);
    assert!(matches!(
        openai(base).complete(request()).await,
        Err(ProviderError::InvalidResponse)
    ));
    handle.join().unwrap();
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["{\"message\":{\"content\":\"ok\"},\"done\":true}\n \r\n\t"],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "ok"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn terminal_markers_finish_without_waiting_for_socket_close() {
    let (base, handle) = keep_open_server(
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
    );
    let response =
        tokio::time::timeout(Duration::from_millis(100), openai(base).complete(request()))
            .await
            .unwrap()
            .unwrap();
    assert_eq!(response.message.content, "ok");
    handle.join().unwrap();
    let (base, handle) = keep_open_server("{\"message\":{\"content\":\"ok\"},\"done\":true}\n");
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = tokio::time::timeout(Duration::from_millis(100), provider.complete(request()))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(response.message.content, "ok");
    handle.join().unwrap();
}

#[tokio::test]
async fn openai_sse_assembles_fragmented_tool_calls() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-7\",\"function\":{\"name\":\"schema_",
            "discovery\",\"arguments\":\"{\\\"schema\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"public\\\"}\"}}]}}]}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let response = openai(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.tool_calls[0].name, "schema_discovery");
    assert_eq!(response.message.tool_calls[0].arguments["schema"], "public");
}

#[tokio::test]
async fn ollama_ndjson_handles_fragmentation_and_requires_done() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"re",
            "ady\"},\"done\":false}\n{\"done\":true}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ready");
    assert!(requests.lock().unwrap()[0].contains("\"stream\":true"));
}

#[tokio::test]
async fn ollama_accepts_terminal_record_without_trailing_newline() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["{\"message\":{\"content\":\"ready\"},\"done\":true}"],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    assert_eq!(
        provider.complete(request()).await.unwrap().message.content,
        "ready"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn retries_before_first_event_but_not_after_partial_stream() {
    let (base, requests, handle) = server(vec![
        Reply {
            status: 429,
            chunks: vec![],
        },
        Reply {
            status: 200,
            chunks: vec![
                "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
            ],
        },
    ]);
    assert_eq!(
        openai(base)
            .complete(request())
            .await
            .unwrap()
            .message
            .content,
        "ok"
    );
    handle.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 2);
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n"],
    }]);
    assert!(matches!(
        openai(base).complete(request()).await,
        Err(ProviderError::InvalidResponse)
    ));
    handle.join().unwrap();
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn cancellation_and_errors_are_sanitized() {
    let (base, _, handle) = server(vec![Reply {
        status: 500,
        chunks: vec!["secret-sentinel"],
    }]);
    let error = openai(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(!error.to_string().contains("secret-sentinel"));
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"slow\"}}]}\n\n"],
    }]);
    let provider = openai(base);
    let token = CancellationToken::new();
    let mut stream = provider.stream(request(), token.clone()).await.unwrap();
    token.cancel();
    assert!(matches!(
        stream.next().await.unwrap(),
        Err(ProviderError::Cancelled)
    ));
    handle.join().unwrap();
}

#[tokio::test]
async fn every_streaming_provider_applies_the_establishment_deadline() {
    use saya_agent::AnthropicProvider;

    let (base, handle) = silent_server();
    let error = OpenAiCompatibleProvider::new(
        ProviderSettings::new("m", Some(format!("{base}/v1")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("timed out"), "{error}");

    let (base, handle) = silent_server();
    let error = AnthropicProvider::new(
        ProviderSettings::new("m", Some(format!("{base}/v1")))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
        Some("k"),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("timed out"), "{error}");

    let (base, handle) = silent_server();
    let error = OllamaProvider::new(
        ProviderSettings::new("m", Some(base))
            .with_retry_delays(Vec::new())
            .with_timeout(Duration::from_millis(20)),
    )
    .unwrap()
    .complete(request())
    .await
    .unwrap_err();
    handle.join().unwrap();
    assert!(error.to_string().contains("timed out"), "{error}");
}

#[tokio::test]
async fn anthropic_stream_reports_stall_on_idle_timeout() {
    use saya_agent::AnthropicProvider;
    let body = "event: message_start\ndata: {\"type\":\"message_start\"}\n\n";
    let (base, handle) = keep_open_server(body);
    let provider = AnthropicProvider::new(
        ProviderSettings::new("test-model", Some(base))
            .with_retry_delays(vec![Duration::ZERO])
            .with_idle_timeout(Duration::from_millis(120)),
        Some("secret-sentinel"),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut stalled = false;
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            stalled = format!("{error:?}").contains("stalled");
            break;
        }
    }
    assert!(stalled, "stall must surface as a stall error");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "idle budget, not socket close, must end the stream"
    );
    handle.join().unwrap();
}

#[tokio::test]
async fn openai_stream_reports_stall_on_idle_timeout() {
    let body = "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n";
    let (base, handle) = keep_open_server(body);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO])
            .with_idle_timeout(Duration::from_millis(120)),
        Some("secret-sentinel"),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut stalled = false;
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            stalled = format!("{error:?}").contains("stalled");
            break;
        }
    }
    assert!(stalled, "stall must surface as a stall error");
    assert!(started.elapsed() < Duration::from_secs(5));
    handle.join().unwrap();
}

#[tokio::test]
async fn ollama_stream_reports_stall_on_idle_timeout() {
    let body = "{\"message\":{\"content\":\"partial\"}}\n";
    let (base, handle) = keep_open_server(body);
    let provider = OllamaProvider::new(
        ProviderSettings::new("test-model", Some(base))
            .with_retry_delays(vec![Duration::ZERO])
            .with_idle_timeout(Duration::from_millis(120)),
    )
    .unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut stalled = false;
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            stalled = format!("{error:?}").contains("stalled");
            break;
        }
    }
    assert!(stalled, "stall must surface as a stall error");
    assert!(started.elapsed() < Duration::from_secs(5));
    handle.join().unwrap();
}

async fn drain(stream: &mut saya_agent::ProviderStream) -> Vec<saya_agent::ProviderEvent> {
    let mut seen = Vec::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            event @ saya_agent::ProviderEvent::Usage(_) => seen.push(event),
            saya_agent::ProviderEvent::Done => {
                seen.push(saya_agent::ProviderEvent::Done);
                break;
            }
            _ => {}
        }
    }
    seen
}

/// Collects every `ReasoningDelta` a stream emits, plus the `Done` sentinel.
/// the per-provider reasoning tests assert reasoning is
/// captured when present and absent when the wire carries none. This drain
/// keeps the reasoning events the general `drain` drops on the floor.
async fn drain_reasoning(
    stream: &mut saya_agent::ProviderStream,
) -> Vec<saya_agent::ProviderEvent> {
    let mut seen = Vec::new();
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            event @ saya_agent::ProviderEvent::ReasoningDelta(_) => seen.push(event),
            saya_agent::ProviderEvent::Done => {
                seen.push(saya_agent::ProviderEvent::Done);
                break;
            }
            _ => {}
        }
    }
    seen
}

#[tokio::test]
async fn anthropic_stream_surfaces_cumulative_token_usage() {
    use saya_agent::{AnthropicProvider, ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":34}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain(&mut stream).await;
    handle.join().unwrap();
    let position = events
        .iter()
        .position(|event| matches!(event, ProviderEvent::Usage(usage) if *usage == TokenUsage::new(12, 34)))
        .expect("usage event with both counters must arrive");
    assert!(
        matches!(events[position + 1], ProviderEvent::Done),
        "usage precedes Done"
    );
}

#[tokio::test]
async fn openai_stream_surfaces_usage_and_requests_it() {
    use saya_agent::{ProviderEvent, TokenUsage};
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6}}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let response = openai(base.clone()).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
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
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":6}}\n\ndata: [DONE]\n\n",
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
    assert!(events.contains(&ProviderEvent::Usage(TokenUsage::new(5, 6))));
}

#[tokio::test]
async fn ollama_stream_surfaces_eval_counts() {
    use saya_agent::{ProviderEvent, TokenUsage};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain(&mut stream).await;
    handle.join().unwrap();
    assert!(events.contains(&ProviderEvent::Usage(TokenUsage::new(9, 11))));
}

#[tokio::test]
async fn length_truncation_is_diagnosable_not_generic() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let error = openai(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    let text = error.to_string();
    assert!(
        text.contains("truncated") && text.contains("output-token"),
        "{text}"
    );
}

/// O1 property 1 (red first): a capped OpenAI response must surface the typed
/// truncation error — not a generic request failure — and carry the partial
/// text the wire had already emitted.
#[tokio::test]
async fn openai_length_truncation_is_a_typed_error_carrying_partial_text() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"partial\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"length\"}]}\n\ndata: [DONE]\n\n",
        ],
    }]);
    let error = openai(base).complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "a capped response must be the typed truncation error, got: {error:?}"
    );
    let saya_agent::ProviderError::OutputTruncated { partial_text, .. } = error else {
        unreachable!("matched above");
    };
    assert_eq!(partial_text, "partial");
}

/// O1 property 4 (Ollama): a final done record with `done_reason: "length"`
/// surfaces the typed truncation error with the partial text. Ollama's
/// `/api/chat` documents `done_reason` on the final record (values `stop`,
/// `length`, `load`, `unload`); `length` mirrors the OpenAI vocabulary and is
/// the reliable signal.
#[tokio::test]
async fn ollama_length_done_reason_is_a_typed_truncation_error() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"partial\"},\"done\":false}\n",
            "{\"done\":true,\"done_reason\":\"length\"}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let error = provider.complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(
        matches!(error, ProviderError::OutputTruncated { .. }),
        "ollama done_reason length must be typed truncation, got: {error:?}"
    );
}

/// O1 property 7: the OpenAI body carries the configured `max_output_tokens`
/// as `max_completion_tokens` (`max_tokens` is deprecated and rejected by
/// newer reasoning models).
#[tokio::test]
async fn openai_body_carries_max_completion_tokens() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec!["data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n"],
    }]);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_max_output_tokens(1234),
        Some("k"),
    )
    .unwrap();
    provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    let sent = requests.lock().unwrap()[0].clone();
    assert!(
        sent.contains("\"max_completion_tokens\":1234"),
        "openai body must carry max_completion_tokens: {sent}"
    );
}

#[tokio::test]
async fn unbounded_frames_fail_at_the_stream_byte_cap() {
    let (base, handle) = byte_server(vec![vec![b'A'; 3 << 20]]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let started = std::time::Instant::now();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let mut capped = false;
    while let Some(event) = stream.next().await {
        if let Err(error) = event {
            capped = error.to_string().contains("size limit");
            break;
        }
    }
    assert!(capped, "boundary-less frames must trip the stream cap");
    assert!(started.elapsed() < Duration::from_secs(5));
    handle.join().unwrap();
}

#[tokio::test]
async fn unreachable_provider_error_names_the_endpoint() {
    use saya_agent::OllamaProvider;
    // Port 1 is reserved and refuses connections: the error must say where
    // we tried to go instead of a bare "network request failed".
    let provider = OllamaProvider::new(ProviderSettings::new(
        "m",
        Some("http://127.0.0.1:1".into()),
    ))
    .unwrap();
    let error = provider.complete(request()).await.unwrap_err().to_string();
    assert!(error.contains("127.0.0.1:1"), "{error}");
    assert!(error.contains("base_url"), "{error}");
}

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

/// a stream with no `reasoning_content`
/// leaves `ChatResponse.reasoning` `None` — no error, no behaviour change
///.
#[tokio::test]
async fn openai_stream_without_reasoning_leaves_it_none() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
            "data: [DONE]\n\n",
        ],
    }]);
    let response = openai(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(response.reasoning, None);
}

/// `thinking_delta.thinking` is parsed
/// into a `ReasoningDelta` event and threaded onto `ChatResponse.reasoning`.
#[tokio::test]
async fn anthropic_stream_captures_thinking_delta() {
    use saya_agent::{AnthropicProvider, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"thinking\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"the column is nullable\"}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"text\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(
        response.reasoning.as_deref(),
        Some("the column is nullable"),
        "thinking_delta.thinking must reach ChatResponse.reasoning"
    );

    // Observe the ReasoningDelta event directly.
    let (base2, _, handle2) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"thinking_delta\",\"thinking\":\"t\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base2)), Some("k")).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_reasoning(&mut stream).await;
    handle2.join().unwrap();
    assert!(
        events.contains(&ProviderEvent::ReasoningDelta("t".into())),
        "thinking_delta must emit a ReasoningDelta: {events:?}"
    );
}

/// a stream with no `thinking` block
/// leaves `ChatResponse.reasoning` `None` — reasoning not reported, no error.
#[tokio::test]
async fn anthropic_stream_without_thinking_leaves_reasoning_none() {
    use saya_agent::AnthropicProvider;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let provider =
        AnthropicProvider::new(ProviderSettings::new("m", Some(base)), Some("k")).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(response.reasoning, None);
}

/// `message.thinking` is parsed into a
/// `ReasoningDelta` and threaded onto `ChatResponse.reasoning`.
#[tokio::test]
async fn ollama_stream_captures_thinking() {
    use saya_agent::{OllamaProvider, ProviderEvent};
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"\",\"thinking\":\"I considered the schema\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(
        response.reasoning.as_deref(),
        Some("I considered the schema"),
        "message.thinking must reach ChatResponse.reasoning"
    );

    // Observe the ReasoningDelta event directly.
    let (base2, _, handle2) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"thinking\":\"t\"},\"done\":false}\n",
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base2))).unwrap();
    let mut stream = provider
        .stream(request(), CancellationToken::new())
        .await
        .unwrap();
    let events = drain_reasoning(&mut stream).await;
    handle2.join().unwrap();
    assert!(
        events.contains(&ProviderEvent::ReasoningDelta("t".into())),
        "message.thinking must emit a ReasoningDelta: {events:?}"
    );
}

/// a chunk with no `thinking` leaves
/// `ChatResponse.reasoning` `None`.
#[tokio::test]
async fn ollama_stream_without_thinking_leaves_reasoning_none() {
    use saya_agent::OllamaProvider;
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "{\"message\":{\"content\":\"ok\"},\"done\":false}\n",
            "{\"done\":true,\"prompt_eval_count\":9,\"eval_count\":11}\n",
        ],
    }]);
    let provider = OllamaProvider::new(ProviderSettings::new("test", Some(base))).unwrap();
    let response = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "ok");
    assert_eq!(response.reasoning, None);
}

/// A loopback Ollama endpoint that answers a cross-host 307 must not have
/// its POST replayed to the redirect target: the default reqwest policy
/// follows up to 10 redirects and replays the body on 307, so an answering
/// redirect could pull the prompt — including database-derived context —
/// off the local classification onto another host without consent.
#[tokio::test]
async fn ollama_does_not_replay_the_post_to_a_cross_host_redirect() {
    use saya_agent::OllamaProvider;
    // Target host: captures any request body that reaches it, then answers a
    // terminal Ollama record.
    let target = TcpListener::bind("127.0.0.1:0").unwrap();
    target.set_nonblocking(true).unwrap();
    let target_base = format!("http://{}", target.local_addr().unwrap());
    let reached = Arc::new(Mutex::new(Vec::new()));
    let reached_copy = reached.clone();
    let target_handle = thread::spawn(move || {
        // Nonblocking accept with a bounded deadline: when the client
        // refuses the redirect (the secure behaviour) nothing ever
        // connects, and the capture must end rather than hang the suite.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::time::Instant::now() < deadline {
            match target.accept() {
                Ok((mut stream, _)) => {
                    target.set_nonblocking(false).ok();
                    reached_copy.lock().unwrap().push(read_request(&mut stream));
                    let body = "{\"message\":{\"content\":\"redirected\"},\"done\":true}\n";
                    write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    )
                    .unwrap();
                    return;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return,
            }
        }
    });
    // Origin host: answers 307 to the target's chat URL.
    let origin = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin_base = format!("http://{}", origin.local_addr().unwrap());
    let origin_handle = thread::spawn(move || {
        let (mut stream, _) = origin.accept().unwrap();
        let _ = read_request(&mut stream);
        let location = format!("{target_base}/api/chat");
        let head = format!(
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.flush().unwrap();
    });
    let provider = OllamaProvider::new(
        ProviderSettings::new("test", Some(origin_base)).with_retry_delays(vec![Duration::ZERO]),
    )
    .unwrap();
    let _ = provider.complete(request()).await;
    origin_handle.join().unwrap();
    target_handle.join().unwrap();
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the Ollama POST to the redirect target"
    );
}

/// Probe establishing what reqwest's default redirect handling does with the
/// headers and body these providers send: `Authorization` (OpenAI's
/// `bearer_auth`) is stripped on a cross-host hop, but the bespoke key
/// headers (`x-api-key`, `x-goog-api-key`) are replayed untouched — and a
/// 307 replays the POST body carrying the prompt. Read against reqwest
/// 0.12's `redirect::remove_sensitive_headers`, which removes exactly
/// `authorization`, `cookie`, `cookie2`, `proxy-authorization`, and
/// `www-authenticate`.
#[tokio::test]
async fn default_client_replays_body_and_bespoke_key_headers_but_strips_authorization() {
    let capture = redirect_capture(
        "/v1/chat/completions",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
    );
    let client = reqwest::Client::builder().build().unwrap();
    let url = format!("{}/v1/chat/completions", capture.origin_base);
    let _ = client
        .post(&url)
        .header("authorization", "Bearer secret-sentinel")
        .header("x-api-key", "secret-sentinel")
        .header("x-goog-api-key", "secret-sentinel")
        .body("{\"messages\":[{\"content\":\"database-derived context\"}]}")
        .send()
        .await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap();
    assert_eq!(
        hits.len(),
        1,
        "the default client must follow the 307 for this probe to say anything"
    );
    let replayed = &hits[0];
    assert!(
        replayed.contains("database-derived context"),
        "a 307 replays the POST body to the redirect target: {replayed}"
    );
    assert!(
        !replayed.to_ascii_lowercase().contains("authorization:"),
        "Authorization is stripped on a cross-host hop: {replayed}"
    );
    assert!(
        replayed.contains("x-api-key: secret-sentinel"),
        "the bespoke x-api-key header travels to the redirect target: {replayed}"
    );
    assert!(
        replayed.contains("x-goog-api-key: secret-sentinel"),
        "the bespoke x-goog-api-key header travels to the redirect target: {replayed}"
    );
}

/// A cross-host 307 must not replay the OpenAI POST to the redirect target:
/// the body carries the prompt (including database-derived context), and the
/// `Authorization: Bearer` key rides the default policy's replayed request
/// only until reqwest strips it — so the prompt disclosure is the finding
/// this test pins, with the key spared by the strip rather than by policy.
#[tokio::test]
async fn openai_does_not_replay_the_post_to_a_cross_host_redirect() {
    let capture = redirect_capture(
        "/v1/chat/completions",
        "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n",
    );
    let provider = openai(capture.origin_base.clone());
    let _ = provider.complete(request()).await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the OpenAI POST to the redirect target"
    );
}

/// A cross-host 307 must not replay the Anthropic POST to the redirect
/// target: reqwest's cross-host strip removes `Authorization` but not the
/// bespoke `x-api-key` header, so both the prompt body and the API key would
/// reach the redirect target under the default policy.
#[tokio::test]
async fn anthropic_does_not_replay_the_post_to_a_cross_host_redirect() {
    use saya_agent::AnthropicProvider;
    let capture = redirect_capture(
        "/messages",
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
    );
    let provider = AnthropicProvider::new(
        ProviderSettings::new("test-model", Some(capture.origin_base.clone()))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap();
    let _ = provider.complete(request()).await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the Anthropic POST (prompt body and x-api-key) to the redirect target"
    );
}

/// A cross-host 307 must not replay the Gemini POST to the redirect target:
/// reqwest's cross-host strip removes `Authorization` but not the bespoke
/// `x-goog-api-key` header, so both the prompt body and the API key would
/// reach the redirect target under the default policy.
#[tokio::test]
async fn gemini_does_not_replay_the_post_to_a_cross_host_redirect() {
    use saya_agent::GeminiProvider;
    let capture = redirect_capture(
        "/v1beta/models/test-model:generateContent",
        r#"{"candidates":[{"content":{"parts":[{"text":"ok"}]}}]}"#,
    );
    let origin_base = capture.origin_base.clone();
    let provider = GeminiProvider::new(
        ProviderSettings::new("test-model", Some(format!("{origin_base}/v1beta")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap();
    let _ = provider.complete(request()).await;
    let (_, reached) = await_capture(capture);
    let hits = reached.lock().unwrap().len();
    assert_eq!(
        hits, 0,
        "a cross-host 307 must not replay the Gemini POST (prompt body and x-goog-api-key) to the redirect target"
    );
}
