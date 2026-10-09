/// Reads one HTTP request from the mock's accepted connection to completion:
/// the headers, then exactly the `Content-Length` body bytes. Copied from
/// `tests/correctness/common.rs`.
fn drain_request(stream: &mut std::net::TcpStream) -> String {
    let (mut buffer, mut chunk) = (Vec::new(), [0_u8; 8192]);
    loop {
        let head = buffer.windows(4).position(|window| window == b"\r\n\r\n");
        if let Some(head) = head {
            let length = String::from_utf8_lossy(&buffer[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if buffer.len() >= head + 4 + length {
                return String::from_utf8_lossy(&buffer).into_owned();
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) | Err(_) => return String::from_utf8_lossy(&buffer).into_owned(),
            Ok(n) => buffer.extend_from_slice(&chunk[..n]),
        }
    }
}

/// A scripted OpenAI-compatible provider: one SSE body per connection in
/// script order (then the text fallback), capturing every request body. The
/// thread returns after a quiet period with no new connection, so `join`
/// never hangs on a run a budget ceiling stopped early.
struct MockProvider {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

fn spawn_mock(responses: Vec<String>) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        listener.set_nonblocking(true).unwrap();
        let mut script = responses.into_iter();
        let mut last_activity = Instant::now();
        loop {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    last_activity = Instant::now();
                    stream.set_nonblocking(false).unwrap();
                    let request = drain_request(&mut stream);
                    captured.lock().unwrap().push(request);
                    let body = script.next().unwrap_or_else(|| text_body(FALLBACK_TEXT));
                    write_response(&mut stream, &body);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if last_activity.elapsed() >= QUIET_AFTER_LAST {
                        return;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("mock accept failed: {error}"),
            }
        }
    });
    MockProvider {
        address,
        bodies,
        handle: Some(handle),
    }
}

fn write_response(stream: &mut std::net::TcpStream, body: &str) {
    use std::io::Write;
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    stream.flush().unwrap();
}

impl MockProvider {
    fn address(&self) -> &str {
        &self.address
    }

    /// Captured request bodies, in script order.
    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    /// Joins the mock thread; always returns (bounded quiet period).
    fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

/// One SSE `data:` frame carrying a single tool call. Bodies are built with
/// `serde_json`, never by hand-escaping.
fn tool_call_body(call_id: &str, name: &str, args: serde_json::Value) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": {
                "tool_calls": [{
                    "index": 0,
                    "id": call_id,
                    "function": {
                        "name": name,
                        "arguments": serde_json::to_string(&args).unwrap(),
                    },
                }],
            },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// One SSE `data:` frame carrying a plain-text final answer.
fn text_body(text: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": { "content": text },
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// One SSE `data:` frame carrying the provider's output-token truncation
/// signal: `finish_reason: "length"`, which the OpenAI wire maps to the
/// typed `OutputTruncated` error the continuation ceiling bounds.
fn truncated_body() -> String {
    let chunk = serde_json::json!({
        "choices": [{
            "delta": { "content": "a partial answer that never completes" },
            "finish_reason": "length",
        }],
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}
