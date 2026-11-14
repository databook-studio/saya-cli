use std::{
    io::Read,
    net::TcpListener,
    sync::{Arc, Mutex},
    thread,
};

/// Reads one HTTP request from the mock's accepted connection to completion:
/// the headers, then exactly the `Content-Length` body bytes — returned as a
/// string so a scenario can assert on what saya sent (the knowledge block, the
/// tool result). The full drain mirrors tests/mvp/common.rs: reqwest sends the
/// body after the headers, and a single short read leaves the socket with
/// unread bytes, so closing it resets the connection mid-request and the
/// client retries, consuming the scripted responses.
pub(crate) fn drain_request(stream: &mut std::net::TcpStream) -> String {
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

/// A scripted OpenAI-compatible provider. One SSE body per connection, in
/// script order; every request body is captured so a scenario can assert on
/// what saya sent (the knowledge block, the tool result). Serves exactly
/// `responses.len()` connections, then stops: an unexpected extra provider
/// call fails loudly (connection refused) rather than being absorbed.
pub(crate) struct MockProvider {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<thread::JoinHandle<()>>,
}

pub(crate) fn spawn_mock(responses: Vec<String>) -> MockProvider {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    let handle = thread::spawn(move || {
        for body in responses {
            let (mut stream, _) = listener.accept().unwrap();
            let request = drain_request(&mut stream);
            captured.lock().unwrap().push(request);
            write_response(&mut stream, &body);
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
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// Captured request bodies, in script order: body[i] is what saya sent on
    /// the round the mock answered with the i-th scripted response. Each
    /// entry is the raw HTTP text (headers + JSON body); use
    /// [`request_json`] to read the JSON part.
    pub(crate) fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    pub(crate) fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.join().unwrap();
        }
    }
}

/// The JSON body of a captured raw HTTP request (headers + body).
pub(crate) fn request_json(raw: &str) -> serde_json::Value {
    let body = raw.rsplit("\r\n\r\n").next().unwrap_or(raw);
    serde_json::from_str(body).unwrap()
}
