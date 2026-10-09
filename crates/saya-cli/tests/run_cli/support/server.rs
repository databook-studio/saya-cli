struct ChildGuard(std::process::Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl std::ops::Deref for ChildGuard {
    type Target = std::process::Child;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for ChildGuard {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

/// One scripted provider response: its SSE body and how long to wait before
/// sending it (a wall-clock budget's trip depends on a slow response).
struct Scripted {
    body: String,
    delay_ms: u64,
}

/// A scripted local provider: one response per connection, in order; the
/// script spent, every further connection is accepted and dropped — a failed
/// call, never a hang. The thread never terminates on its own (it stays in
/// `accept`), so tests drop the handle rather than joining it: the thread
/// dies with the test process and can never block a test's exit. The
/// returned flag flips once the first connection has been read, so a test
/// can wait until the binary is provably inside a provider call.
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
                    thread::sleep(std::time::Duration::from_millis(scripted.delay_ms));
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

/// A provider response held by a test-controlled channel after the request
/// has arrived. This lets cancellation tests distinguish local ownership
/// from an elapsed mock delay.
#[cfg(unix)]
fn held_mock(body: String) -> (String, Arc<AtomicBool>, mpsc::Sender<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let arrived = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&arrived);
    let (release, wait) = mpsc::channel();
    thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 32_768];
            let _ = stream.read(&mut request);
            flag.store(true, Ordering::SeqCst);
            let _ = wait.recv();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
    });
    (address, arrived, release)
}

fn counted_drop_server() -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&requests);
    thread::spawn(move || {
        while let Ok((mut stream, _)) = listener.accept() {
            let mut request = [0_u8; 32_768];
            let _ = stream.read(&mut request);
            count.fetch_add(1, Ordering::SeqCst);
        }
    });
    (address, requests)
}

/// Waits until the mock has served its first connection, with a generous
/// bound so a wedged child fails the test instead of hanging it.
#[cfg(unix)]
fn wait_ready(flag: &Arc<AtomicBool>) {
    for _ in 0..200 {
        if flag.load(Ordering::SeqCst) {
            return;
        }
        thread::sleep(Duration::from_millis(50));
    }
    panic!("the binary never reached its first provider call");
}

/// One streaming response carrying `content` as the message's text.
fn sse(content: &str) -> String {
    let chunk = serde_json::json!({"choices": [{"delta": {"content": content}}]});
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// The planner's answer: one JSON plan object, `steps` in order — the shape
/// `plan/prompt.rs` demands and the engine parses.
fn plan_body(goals: &[&str]) -> String {
    let steps = goals
        .iter()
        .map(|goal| {
            serde_json::json!({
                "goal": goal,
                "capabilities": {},
                "budget": serde_json::Value::Null,
                "expects": [],
                "endpoint": serde_json::Value::Null
            })
        })
        .collect::<Vec<_>>();
    let plan = serde_json::json!({"steps": steps});
    sse(&plan.to_string())
}

/// The planner's answer with per-step capabilities, for plans that ask for
/// scopes beyond what `--allow` granted.
fn plan_body_with_capabilities(steps: &[(&str, serde_json::Value)]) -> String {
    let steps = steps
        .iter()
        .map(|(goal, capabilities)| {
            serde_json::json!({
                "goal": goal,
                "capabilities": capabilities,
                "budget": serde_json::Value::Null,
                "expects": [],
                "endpoint": serde_json::Value::Null
            })
        })
        .collect::<Vec<_>>();
    let plan = serde_json::json!({"steps": steps});
    sse(&plan.to_string())
}
