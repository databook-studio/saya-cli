use std::sync::Arc;

/// A provider that answers every request with one sentence, streamed: the
/// surface's own turn has something to paint, and nothing reaches a network.
///
/// The stream is slow on purpose — four deltas a hundred milliseconds apart —
/// so a test can press Esc while the turn is still running. A test that only
/// wants the answer waits for it; nothing here decides what the surface does.
struct MockProvider {
    address: String,
    requests: Arc<std::sync::Mutex<u64>>,
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl MockProvider {
    /// The sentence every answer ends in: the words a test waits for.
    const ANSWER: &'static str = "two plus two is four";

    fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("the mock provider binds");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        listener
            .set_nonblocking(true)
            .expect("the listener becomes nonblocking");
        let requests = Arc::new(std::sync::Mutex::new(0_u64));
        let counted = Arc::clone(&requests);
        let (stop, stopped) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || {
            loop {
                let stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if stopped.try_recv().is_ok() {
                            return;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    }
                    Err(_) => return,
                };
                if !answer_one(stream, &counted) {
                    return;
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    /// The base URL the launch is pointed at.
    fn base_url(&self) -> String {
        format!("{}/v1", self.address)
    }

    /// How many requests reached the provider so far.
    fn requests(&self) -> u64 {
        *self.requests.lock().unwrap_or_else(|error| error.into_inner())
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Reads one whole request and answers it with the streamed sentence. False
/// when the connection ended before a request arrived.
fn answer_one(mut stream: std::net::TcpStream, counted: &Arc<std::sync::Mutex<u64>>) -> bool {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    if drain_request(&mut stream).is_empty() {
        return true;
    }
    *counted.lock().unwrap_or_else(|error| error.into_inner()) += 1;
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut body = String::new();
    for chunk in split_answer(MockProvider::ANSWER) {
        let delta = serde_json::json!({"choices": [{"delta": {"content": chunk}}]});
        body.push_str(&format!("data: {delta}\n\n"));
        // One delta per pause: a test that means to stop the turn has a turn
        // still running when its key arrives.
        std::thread::sleep(Duration::from_millis(150));
    }
    body.push_str("data: [DONE]\n\n");
    let head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = write!(stream, "{head}{body}");
    let _ = stream.flush();
    true
}

/// The answer in the deltas it is streamed as, each a word and a space.
fn split_answer(answer: &str) -> Vec<String> {
    answer
        .split_inclusive(' ')
        .map(str::to_owned)
        .collect::<Vec<String>>()
}

/// Reads the request head and body, and returns the body.
fn drain_request(stream: &mut std::net::TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let length = String::from_utf8_lossy(&bytes[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + length {
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned();
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return String::new(),
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(_) => return String::new(),
        }
    }
}
