// The scripted provider one ask row runs against: one SSE reply per
// connection, in script order, each held for its step's own delay — the held
// reply is what a turn's own elapsed budget runs out against.

/// One scripted reply, held for `delay` inside the source before it is written.
pub(crate) struct Step {
    pub(crate) delay: Duration,
    pub(crate) body: String,
}

/// A reply the source writes at once.
pub(crate) fn reply(body: String) -> Step {
    Step {
        delay: Duration::ZERO,
        body,
    }
}

/// A reply the source holds: no byte reaches the run until `delay` passed.
pub(crate) fn held(body: String, delay: Duration) -> Step {
    Step { delay, body }
}

/// A scripted OpenAI-compatible provider: one reply per connection, in script
/// order, and every request body captured in the order it arrived.
pub(crate) struct Scripted {
    address: String,
    bodies: Arc<Mutex<Vec<String>>>,
}

pub(crate) fn serve(steps: Vec<Step>) -> Scripted {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bodies);
    thread::spawn(move || {
        for step in steps {
            let (mut stream, _) = listener.accept().unwrap();
            let request = drain(&mut stream);
            captured.lock().unwrap().push(request);
            thread::sleep(step.delay);
            write_response(&mut stream, &step.body);
        }
    });
    Scripted { address, bodies }
}

impl Scripted {
    pub(crate) fn address(&self) -> &str {
        &self.address
    }

    /// The request bodies the provider was asked with, in arrival order.
    pub(crate) fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }
}

/// Reads one HTTP request to completion: its headers, then exactly the
/// `Content-Length` body bytes.
fn drain(stream: &mut TcpStream) -> String {
    let (mut buffer, mut chunk) = (Vec::new(), [0_u8; 8192]);
    loop {
        if let Some(head) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
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

fn write_response(stream: &mut TcpStream, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )
    .unwrap();
    stream.flush().unwrap();
}

// The wire bodies the rows script.
include!("provider/frames.rs");
