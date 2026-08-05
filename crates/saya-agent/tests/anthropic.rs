use saya_agent::{
    AnthropicProvider, ChatMessage, ChatProvider, ChatRequest, ProviderSettings, ToolDefinition,
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
                "HTTP/1.1 {} OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
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
    ChatRequest {
        model: "claude-x".into(),
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

fn anthropic(base: String) -> AnthropicProvider {
    AnthropicProvider::new(
        ProviderSettings::new("claude-x", Some(format!("{base}/v1")))
            .with_retry_delays(vec![Duration::ZERO]),
        Some("key-sentinel"),
    )
    .unwrap()
}

#[tokio::test]
async fn test_anthropic_text_completion() {
    let (base, requests, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hel\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"lo\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let response = anthropic(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.content, "Hello");
    let sent = &requests.lock().unwrap()[0];
    assert!(sent.contains("x-api-key: key-sentinel"));
    assert!(sent.contains("anthropic-version"));
    assert!(sent.contains("\"stream\":true"));
    assert!(sent.contains("input_schema"));
}

#[tokio::test]
async fn test_anthropic_tool_call() {
    let (base, _, handle) = server(vec![Reply {
        status: 200,
        chunks: vec![
            "event: message_start\ndata: {\"type\":\"message_start\"}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"schema_discovery\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"connection\\\":\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"\\\"warehouse\\\"}\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
        ],
    }]);
    let response = anthropic(base).complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(response.message.tool_calls[0].name, "schema_discovery");
    assert_eq!(
        response.message.tool_calls[0].arguments["connection"],
        "warehouse"
    );
}
