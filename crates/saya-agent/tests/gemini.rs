use saya_agent::{
    ChatMessage, ChatProvider, ChatRequest, GeminiProvider, ProviderSettings, ToolDefinition,
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
    ChatRequest {
        model: "gemini-x".into(),
        messages: vec![
            ChatMessage::text("system", "system prompt"),
            ChatMessage::text("user", "hello"),
        ],
        tools: vec![ToolDefinition {
            name: "schema_discovery".into(),
            description: "schema".into(),
            read_only: true,
            parameters: serde_json::json!({"type":"object"}),
            requires_approval: false,
        }],
    }
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
