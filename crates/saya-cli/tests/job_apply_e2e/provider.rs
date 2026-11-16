//! A scripted OpenAI-compatible provider for the job end-to-end test: one
//! reply per accepted request, each computed from the request body it answers,
//! so the edit sees the read's digest.

use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

/// One scripted provider reply.
pub(super) enum Round {
    Tool {
        name: &'static str,
        arguments: serde_json::Value,
    },
    Text(&'static str),
}

/// One scripted provider round: computes its reply from the request body.
pub(super) type RoundFn = Box<dyn FnMut(&str) -> Round + Send>;

/// Serves one scripted reply per accepted request, then stops.
pub(super) fn scripted(rounds: Vec<RoundFn>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("scripted provider binds");
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&bodies);
    thread::spawn(move || {
        for (index, mut round) in rounds.into_iter().enumerate() {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut raw = Vec::new();
            let mut buffer = [0_u8; 8192];
            let mut wanted = None;
            loop {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => {
                        raw.extend_from_slice(&buffer[..size]);
                        if wanted.is_none()
                            && let Some(end) = raw.windows(4).position(|w| w == b"\r\n\r\n")
                        {
                            let head = String::from_utf8_lossy(&raw[..end]);
                            wanted = head
                                .lines()
                                .find_map(|line| {
                                    line.strip_prefix("content-length: ")
                                        .or_else(|| line.strip_prefix("Content-Length: "))
                                })
                                .and_then(|value| value.trim().parse::<usize>().ok())
                                .map(|length| end + 4 + length);
                        }
                        if wanted.is_some_and(|wanted| raw.len() >= wanted) {
                            break;
                        }
                    }
                }
            }
            let request = String::from_utf8_lossy(&raw).into_owned();
            seen.lock().unwrap().push(request.clone());
            let chunk = match round(&request) {
                Round::Tool { name, arguments } => serde_json::json!({
                    "choices": [{"delta": {"tool_calls": [{
                        "index": 0,
                        "id": format!("call_{index}"),
                        "function": {"name": name, "arguments": arguments.to_string()}
                    }]}}]
                }),
                Round::Text(text) => serde_json::json!({"choices": [{"delta": {"content": text}}]}),
            };
            let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
    });
    (address, bodies)
}

fn request_json(raw: &str) -> serde_json::Value {
    let (_, body) = raw.split_once("\r\n\r\n").expect("HTTP request has a body");
    serde_json::from_str(body).expect("request body parses as JSON")
}

/// The `digest` the read tool reported in the request's last tool result.
pub(super) fn digest_from(raw: &str) -> String {
    let request = request_json(raw);
    for message in request["messages"].as_array().expect("request messages") {
        if message["role"] != "tool" {
            continue;
        }
        let Some(content) = message["content"].as_str() else {
            continue;
        };
        if let Some(at) = content.find("\"digest\":\"") {
            let hex: String = content[at + 10..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if hex.len() == 64 {
                return hex;
            }
        }
    }
    panic!("no tool result carries a digest: {request}");
}
