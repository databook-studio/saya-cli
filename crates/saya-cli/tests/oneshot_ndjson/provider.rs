//! A scripted OpenAI-compatible provider for the `-p` ndjson tests: one reply
//! per accepted request, then the listener stops.

use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::Duration,
};

/// One scripted provider reply: a tool call or a final answer.
pub(super) enum Round {
    Tool {
        name: &'static str,
        arguments: serde_json::Value,
    },
    Text(&'static str),
}

/// Serves one scripted reply per accepted request, in order, then stops.
pub(super) fn scripted(rounds: Vec<Box<dyn FnMut() -> Round + Send>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("scripted provider binds");
    let address = format!("http://{}", listener.local_addr().unwrap());
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
            let chunk = match round() {
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
    address
}

/// Every parsed ndjson line the run printed on stdout.
pub(super) fn ndjson(stdout: &str) -> Vec<serde_json::Value> {
    stdout
        .lines()
        .filter(|line| line.starts_with('{'))
        .map(|line| serde_json::from_str(line).expect("every ndjson line parses"))
        .collect()
}

/// The one line of `event` naming tool `name`, if the run printed it.
pub(super) fn event<'a>(
    lines: &'a [serde_json::Value],
    event: &str,
    name: &str,
) -> Option<&'a serde_json::Value> {
    lines
        .iter()
        .find(|line| line["event"] == event && line["name"] == name)
}
