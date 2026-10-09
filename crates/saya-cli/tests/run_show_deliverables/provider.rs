/// One scripted provider response: its SSE body and how long to wait before
/// sending it.
struct Scripted {
    body: String,
    delay_ms: u64,
}

/// A scripted local provider: one response per connection, in order; the
/// script spent, every further connection is accepted and dropped — a failed
/// call, never a hang. The thread never terminates on its own; tests drop
/// the handle and it dies with the test process (`run_cli.rs` is the
/// pattern). The flag flips once the first connection has been read.
fn mock(script: Vec<Scripted>) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let first_arrived = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&first_arrived);
    thread::spawn(move || {
        let mut first = true;
        for scripted in script {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0_u8; 32_768];
                let _ = stream.read(&mut request);
                if first {
                    first = false;
                    flag.store(true, Ordering::SeqCst);
                }
                if scripted.delay_ms > 0 {
                    thread::sleep(Duration::from_millis(scripted.delay_ms));
                }
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: \
                     text/event-stream\r\nConnection: close\r\n\r\n{}",
                    scripted.body.len(),
                    scripted.body
                );
                let _ = stream.flush();
            }
        }
        while let Ok((stream, _)) = listener.accept() {
            drop(stream);
        }
    });
    (address, first_arrived)
}

/// One streaming response carrying `content` as the message's text — no tool
/// calls, so the loop terminates.
fn sse(content: &str) -> String {
    let chunk = serde_json::json!({"choices": [{"delta": {"content": content}}]});
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// The planner's answer: one step with `workspace_write` and the declared
/// expected outputs — the shape `plan/prompt.rs` demands.
fn plan_with_expects(expects: &[(&str, &str)]) -> String {
    let step = serde_json::json!({
        "goal": "produce the declared artifacts",
        "capabilities": {"workspace_write": true},
        "budget": serde_json::Value::Null,
        "expects": expects
            .iter()
            .map(|(name, description)| {
                serde_json::json!({"name": name, "description": description})
            })
            .collect::<Vec<_>>(),
        "endpoint": serde_json::Value::Null
    });
    sse(&serde_json::json!({"steps": [step]}).to_string())
}

/// One streaming response carrying a single `workspace_write` tool call.
fn write_call(path: &str, content: &str) -> String {
    let chunk = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": "call_write",
            "function": {
                "name": "workspace_write",
                "arguments": serde_json::json!({"path": path, "content": content}).to_string()
            }
        }]}}]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}
