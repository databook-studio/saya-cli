struct MockProvider {
    address: String,
    requests: Arc<Mutex<usize>>,
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockProvider {
    fn spawn_with_follow_up() -> Self {
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
                return;
            };
            drain_request(&mut stream);
            *recorded.lock().unwrap() += 1;
            if stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .is_err()
            {
                return;
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
                return;
            }
            if let Ok(mut stream) = accept_request(&listener, &stopped) {
                drain_request(&mut stream);
                *recorded.lock().unwrap() += 1;
                if stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .is_ok()
                {
                    let response = answer_response();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    );
                    let _ = stream.flush();
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

    fn requests(&self) -> usize {
        *self.requests.lock().unwrap()
    }

    fn join(&mut self) {
        let _ = self.stop.send(());
        self.thread
            .take()
            .expect("provider thread joins")
            .join()
            .expect("provider joins cleanly");
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
