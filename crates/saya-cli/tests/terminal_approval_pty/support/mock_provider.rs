struct MockProvider {
    address: String,
    requests: Arc<Mutex<usize>>,
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<Option<String>>>,
}

impl MockProvider {
    fn spawn() -> Self {
        Self::spawn_inner(false)
    }

    fn spawn_with_follow_up() -> Self {
        Self::spawn_inner(true)
    }

    fn spawn_inner(follow_up: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(0));
        let recorded = Arc::clone(&requests);
        let (stop, stopped) = mpsc::channel();
        let thread = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("provider listener becomes nonblocking");
            let Ok(mut stream) = accept_request(&listener, &stopped) else {
                return None;
            };
            drain_request(&mut stream);
            *recorded.lock().unwrap() += 1;
            if stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .is_err()
            {
                return None;
            }
            let response = tool_response();
            if write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .is_err()
                || stream.flush().is_err()
            {
                return None;
            }
            if !follow_up {
                return None;
            }
            let Ok(mut stream) = accept_request(&listener, &stopped) else {
                return None;
            };
            let request = drain_request(&mut stream);
            *recorded.lock().unwrap() += 1;
            if stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .is_err()
            {
                return None;
            }
            let response = answer_response();
            if write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .is_err()
                || stream.flush().is_err()
            {
                return None;
            }
            Some(request)
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> usize {
        *self.requests.lock().unwrap()
    }

    fn join(&mut self) {
        let _ = self.stop.send(());
        assert!(self.thread.take().unwrap().join().unwrap().is_none());
    }

    fn join_follow_up(&mut self) -> String {
        let _ = self.stop.send(());
        self.thread
            .take()
            .unwrap()
            .join()
            .unwrap()
            .expect("the provider receives the paired tool result")
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

fn accept_request(listener: &TcpListener, stop: &mpsc::Receiver<()>) -> std::io::Result<TcpStream> {
    // The mock must outlive a slow child start; the test's overall DEADLINE is the real bound.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit O_NONBLOCK from a non-blocking listener on macOS; drain_request assumes blocking reads under its 2s read timeout.
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.try_recv().is_ok() {
                    return Err(std::io::Error::other("provider stopped"));
                }
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "provider request deadline elapsed",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn drain_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("provider request read timeout");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let content_length = String::from_utf8_lossy(&bytes[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + content_length {
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + content_length])
                    .into_owned();
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return String::new(),
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return String::new();
                }
            }
            Err(_) => return String::new(),
        }
    }
}

fn tool_response() -> String {
    let chunk = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": "call-1",
            "function": {
                "name": "bounded_sql_query",
                "arguments": "{\"sql\":\"SELECT 1\"}"
            }
        }]}}]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

fn answer_response() -> String {
    "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n".into()
}

fn paired_tool_result(request: &str) -> String {
    let request: serde_json::Value =
        serde_json::from_str(request).expect("provider request is JSON");
    let results: Vec<_> = request["messages"]
        .as_array()
        .expect("provider request carries messages")
        .iter()
        .filter(|message| message["role"] == "tool" && message["tool_call_id"] == "call-1")
        .collect();
    assert_eq!(results.len(), 1, "one paired tool result for call-1");
    results[0]["content"]
        .as_str()
        .expect("paired tool result content is text")
        .to_owned()
}
