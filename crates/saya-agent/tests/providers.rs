use futures_util::StreamExt;
use saya_agent::{
    CancellationToken, ChatMessage, ChatProvider, ChatRequest, OllamaProvider,
    OpenAiCompatibleProvider, ProviderError, ProviderSettings, ToolDefinition,
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
        let length: usize = chunks.iter().map(Vec::len).sum();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n").unwrap();
        for chunk in chunks {
            stream.write_all(&chunk).unwrap();
            stream.flush().unwrap();
        }
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
    ChatRequest {
        model: "test-model".into(),
        messages: vec![ChatMessage::text("user", "hello")],
        tools: vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            requires_approval: false,
        }],
    }
}
fn openai(base: String) -> OpenAiCompatibleProvider {
    OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("secret-sentinel"),
    )
    .unwrap()
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
