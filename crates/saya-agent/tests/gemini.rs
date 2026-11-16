use saya_agent::{
    ChatMessage, ChatRequest, GeminiProvider, LocalStateEffect, ProviderSettings, ToolDefinition,
    ToolEffect,
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

#[path = "gemini/completion.rs"]
mod completion;
#[path = "gemini/stream_truncation.rs"]
mod stream_truncation;
#[path = "gemini/timeouts.rs"]
mod timeouts;
#[path = "gemini/truncation.rs"]
mod truncation;
