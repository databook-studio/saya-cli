use saya_agent::{
    ChatMessage, ChatProvider, ChatRequest, OllamaProvider, OpenAiCompatibleProvider,
    ProviderSettings, ToolDefinition,
};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{Arc, Mutex},
    thread,
};

fn mock_server(response: &'static str) -> (String, Arc<Mutex<String>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let request = Arc::new(Mutex::new(String::new()));
    let captured = request.clone();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let body = read_request(&mut stream);
        *captured.lock().unwrap() = body;
        let bytes = response.as_bytes();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", bytes.len(), response).unwrap();
    });
    (address, request, handle)
}

fn mock_sequence(
    responses: Vec<&'static str>,
) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let handle = thread::spawn(move || {
        for response in responses {
            let (mut stream, _) = listener.accept().unwrap();
            captured.lock().unwrap().push(read_request(&mut stream));
            let bytes = response.as_bytes();
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}", bytes.len(), response).unwrap();
        }
    });
    (address, requests, handle)
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let header_end;
    loop {
        let count = stream.read(&mut buffer).unwrap();
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
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

#[tokio::test]
async fn openai_compatible_provider_posts_tools_and_parses_tool_calls() {
    let response = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"call-7","type":"function","function":{"name":"schema_discovery","arguments":"{}"}}]}}]}"#;
    let (base, captured, handle) = mock_server(response);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1"))),
        Some("secret-sentinel"),
    )
    .unwrap();
    let result = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(result.message.tool_calls[0].name, "schema_discovery");
    let sent = captured.lock().unwrap();
    assert!(sent.starts_with("POST /v1/chat/completions"));
    assert!(sent.contains("Bearer secret-sentinel"));
    assert!(sent.contains("schema_discovery"));
}

#[tokio::test]
async fn ollama_provider_posts_non_streaming_chat_and_parses_text() {
    let response = r#"{"message":{"role":"assistant","content":"ready"}}"#;
    let (base, captured, handle) = mock_server(response);
    let provider = OllamaProvider::new(ProviderSettings::new("test-model", Some(base))).unwrap();
    let result = provider.complete(request()).await.unwrap();
    handle.join().unwrap();
    assert_eq!(result.message.content, "ready");
    let sent = captured.lock().unwrap();
    assert!(sent.starts_with("POST /api/chat"));
    assert!(sent.contains("\"stream\":false"));
}

#[tokio::test]
async fn ollama_tool_follow_up_uses_native_arguments_objects() {
    let first = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"schema_discovery","arguments":{}}}]}}"#;
    let second = r#"{"message":{"role":"assistant","content":"schema ready"}}"#;
    let (base, requests, handle) = mock_sequence(vec![first, second]);
    let provider = OllamaProvider::new(ProviderSettings::new("test-model", Some(base))).unwrap();
    let first_result = provider.complete(request()).await.unwrap();
    let follow_up = ChatRequest {
        model: "test-model".into(),
        messages: vec![
            ChatMessage {
                role: "assistant".into(),
                content: String::new(),
                tool_calls: first_result.message.tool_calls,
                tool_call_id: None,
            },
            ChatMessage {
                role: "tool".into(),
                content: "{\"schema\":{}}".into(),
                tool_calls: Vec::new(),
                tool_call_id: Some("call-0".into()),
            },
        ],
        tools: request().tools,
    };
    assert_eq!(
        provider.complete(follow_up).await.unwrap().message.content,
        "schema ready"
    );
    handle.join().unwrap();
    let captured = requests.lock().unwrap();
    assert_eq!(captured.len(), 2);
    assert!(
        captured[1].contains("\"arguments\":{}"),
        "unexpected native payload: {}",
        captured[1]
    );
    assert!(!captured[1].contains("\\\"arguments\\\":"));
    assert!(!captured[1].contains("\"tool_calls\":[{\"id\""));
}

#[tokio::test]
async fn provider_errors_do_not_include_authorization_material() {
    let (base, _, handle) = mock_server(r#"{"error":"secret-sentinel"}"#);
    let provider = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(base)),
        Some("secret-sentinel"),
    )
    .unwrap();
    let error = provider.complete(request()).await.unwrap_err();
    handle.join().unwrap();
    assert!(!error.to_string().contains("secret-sentinel"));
}

#[tokio::test]
async fn openai_and_ollama_reject_empty_assistant_messages() {
    let (base, _, openai_handle) = mock_server(r#"{"choices":[{"message":{"content":null}}]}"#);
    let openai = OpenAiCompatibleProvider::new(
        ProviderSettings::new("test-model", Some(format!("{base}/v1"))),
        None,
    )
    .unwrap();
    assert!(matches!(
        openai.complete(request()).await,
        Err(saya_agent::ProviderError::InvalidResponse)
    ));
    openai_handle.join().unwrap();

    let (base, _, ollama_handle) = mock_server(r#"{"message":{"role":"assistant","content":""}}"#);
    let ollama = OllamaProvider::new(ProviderSettings::new("test-model", Some(base))).unwrap();
    assert!(matches!(
        ollama.complete(request()).await,
        Err(saya_agent::ProviderError::InvalidResponse)
    ));
    ollama_handle.join().unwrap();
}
