//! OpenRouter attribution is a header decision, so the test resolves
//! `openrouter.ai` to a loopback listener: the request a provider really sends
//! is captured, without touching DNS or the network.

use super::{AnthropicProvider, OpenAiCompatibleProvider, ProviderSettings};
use crate::{ChatMessage, ChatProvider, ChatRequest};
use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpListener},
    thread,
};

/// One scripted HTTP exchange: returns the base URL, the captured request, and
/// the server thread.
fn serve(reply: &'static str) -> (SocketAddr, thread::JoinHandle<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let request = read_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{reply}",
            reply.len()
        )
        .unwrap();
        request
    });
    (addr, handle)
}

fn read_request(stream: &mut std::net::TcpStream) -> String {
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
    let header = String::from_utf8_lossy(&bytes[..header_end]).into_owned();
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

/// A client that resolves the public OpenRouter name to the loopback listener.
fn openrouter_client(addr: SocketAddr) -> reqwest::Client {
    reqwest::Client::builder()
        .resolve("openrouter.ai", addr)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

fn request() -> ChatRequest {
    ChatRequest::new("test-model", vec![ChatMessage::text("user", "hello")])
}

const OPENAI_REPLY: &str =
    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\ndata: [DONE]\n\n";
const ANTHROPIC_REPLY: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":1}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hi\"}}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{},\"usage\":{\"output_tokens\":1}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

fn lower(request: &str) -> String {
    request.to_ascii_lowercase()
}

fn assert_attribution(request: &str) {
    let request = lower(request);
    assert!(
        request.contains("user-agent: saya-cli/"),
        "every request names the client: {request}"
    );
    assert!(
        request.contains("http-referer: https://github.com/databook-studio/saya-cli"),
        "an OpenRouter request carries the app referrer: {request}"
    );
    assert!(
        request.contains("x-title: saya cli"),
        "an OpenRouter request carries the display name: {request}"
    );
}

/// Both routes — the OpenAI-style and the Anthropic-style — send the two
/// attribution headers when the base URL's host is `openrouter.ai`.
#[tokio::test]
async fn both_routes_send_attribution_headers_to_openrouter() {
    let (addr, handle) = serve(OPENAI_REPLY);
    let provider = OpenAiCompatibleProvider::with_client(
        ProviderSettings::new("m", Some("http://openrouter.ai/api/v1".into())),
        Some("k"),
        openrouter_client(addr),
    )
    .unwrap();
    provider.complete(request()).await.unwrap();
    assert_attribution(&handle.join().unwrap());

    let (addr, handle) = serve(ANTHROPIC_REPLY);
    let provider = AnthropicProvider::with_client(
        ProviderSettings::new("m", Some("http://openrouter.ai/v1".into())),
        Some("k"),
        openrouter_client(addr),
    )
    .unwrap();
    provider.complete(request()).await.unwrap();
    assert_attribution(&handle.join().unwrap());
}

/// The opt-out removes both attribution headers and keeps the identity one.
#[tokio::test]
async fn opting_out_removes_attribution_headers() {
    let (addr, handle) = serve(OPENAI_REPLY);
    let provider = OpenAiCompatibleProvider::with_client(
        ProviderSettings::new("m", Some("http://openrouter.ai/api/v1".into()))
            .with_app_attribution(false),
        Some("k"),
        openrouter_client(addr),
    )
    .unwrap();
    provider.complete(request()).await.unwrap();
    let request = lower(&handle.join().unwrap());
    assert!(request.contains("user-agent: saya-cli/"), "{request}");
    assert!(!request.contains("http-referer:"), "{request}");
    assert!(!request.contains("x-title:"), "{request}");
}

/// A different host never receives the attribution headers, whatever the
/// setting says.
#[tokio::test]
async fn a_non_openrouter_host_never_gets_attribution_headers() {
    let (addr, handle) = serve(OPENAI_REPLY);
    let provider = OpenAiCompatibleProvider::with_client(
        ProviderSettings::new("m", Some(format!("http://{addr}/v1"))),
        Some("k"),
        reqwest::Client::new(),
    )
    .unwrap();
    provider.complete(request()).await.unwrap();
    let request = lower(&handle.join().unwrap());
    assert!(request.contains("user-agent: saya-cli/"), "{request}");
    assert!(!request.contains("http-referer:"), "{request}");
    assert!(!request.contains("x-title:"), "{request}");
}
