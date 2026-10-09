/// One scripted provider reply: a tool call or the final text.
pub enum Round {
    Tool {
        name: &'static str,
        arguments: serde_json::Value,
    },
    Text(&'static str),
    /// A held model stream: headers WITHOUT a content length, one content
    /// chunk (the marker), flush, signal, then block until the client goes
    /// away — the pattern of tests/mvp/session.rs, so the test can cancel a
    /// mid-request child deterministically (never by sleeping).
    Hold(&'static str),
    /// Every attempt answers HTTP 500: the provider path that fails a turn.
    Fail500,
}

/// A round is computed from the request body it answers — round N's body
/// already carries round N-1's tool results (the read's `digest` for round 2).
pub type RoundFn = Box<dyn FnMut(&str) -> Round + Send>;

pub struct Scripted {
    pub address: String,
    /// Every raw HTTP request the provider received, in order.
    pub bodies: Arc<Mutex<Vec<String>>>,
    /// Set once a hold round has flushed its first chunk: the client is
    /// mid-request and the test may cancel.
    pub held: Arc<AtomicBool>,
}

fn sse(round: &Round, index: usize) -> String {
    let chunk = match round {
        Round::Tool { name, arguments } => serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": format!("call_{index}"),
                "function": {"name": name, "arguments": arguments.to_string()}
            }]}}]
        }),
        Round::Text(text) => serde_json::json!({"choices": [{"delta": {"content": text}}]}),
        // The hold and failure rounds are answered in the serve loop before
        // the SSE body is built; they never reach this shaping.
        Round::Hold(_) | Round::Fail500 => serde_json::Value::Null,
    };
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// Serves one scripted reply per accepted request (pattern of
/// tests/chart_cleanup.rs), then stops. Extra requests are not answered.
pub fn script(rounds: Vec<RoundFn>) -> Scripted {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&bodies);
    let held = Arc::new(AtomicBool::new(false));
    let held_flag = Arc::clone(&held);
    thread::spawn(move || {
        for (index, mut round) in rounds.into_iter().enumerate() {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut raw = Vec::new();
            let mut buf = [0_u8; 8192];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => raw.extend_from_slice(&buf[..size]),
                }
            }
            let request = String::from_utf8_lossy(&raw).into_owned();
            seen.lock().unwrap().push(request.clone());
            match round(&request) {
                Round::Hold(marker) => {
                    let chunk = serde_json::json!({"choices": [{"delta": {"content": marker}}]});
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {chunk}\n\n"
                    );
                    let _ = stream.flush();
                    held_flag.store(true, Ordering::SeqCst);
                    let _ = stream.read_to_end(&mut Vec::new());
                }
                Round::Fail500 => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.flush();
                }
                round => {
                    let body = sse(&round, index);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.flush();
                }
            }
        }
    });
    Scripted {
        address,
        bodies,
        held,
    }
}

/// Blocks until a hold round has flushed its first chunk: the client is
/// mid-request and the test may cancel. Panics at the deadline.
pub fn wait_until_holding(scripted: &Scripted, deadline: Duration) {
    let start = Instant::now();
    while !scripted.held.load(Ordering::SeqCst) {
        if start.elapsed() >= deadline {
            panic!("the hold round never started streaming");
        }
        thread::sleep(Duration::from_millis(5));
    }
}
