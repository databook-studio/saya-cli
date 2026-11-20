//! The scripted provider the turn-per-message tests drive: one request per
//! root turn, answered with a final text. It can hold its first answer, so a
//! turn that is still running is observable while it runs, and it keeps every
//! request body it received.

use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

/// Load ceiling, not a product bound: how long a held turn waits for its test
/// before the provider answers anyway.
const HOLD: Duration = Duration::from_secs(60);

/// A local scripted provider: every request is one root turn, answered with a
/// final text. The first answer can be held, so a turn that is still running
/// is observable while it runs.
pub(super) struct Scripted {
    pub(super) endpoint: String,
    bodies: Arc<Mutex<Vec<String>>>,
    arrived: Arc<AtomicUsize>,
    held: Arc<AtomicBool>,
    released: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Scripted {
    pub(super) fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies: Arc<Mutex<Vec<String>>> = Arc::default();
        let arrived = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(AtomicBool::new(false));
        let released = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, seen, holding, open, done) = (
            Arc::clone(&bodies),
            Arc::clone(&arrived),
            Arc::clone(&held),
            Arc::clone(&released),
            Arc::clone(&stop),
        );
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                let turn = seen.fetch_add(1, Ordering::AcqRel);
                captured.lock().unwrap().push(body);
                if holding.load(Ordering::Acquire) && turn == 0 {
                    let deadline = Instant::now() + HOLD;
                    while !open.load(Ordering::Acquire) && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                let response = answer(&format!("turn {turn} answered"));
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{response}",
                    response.len()
                );
                let _ = stream.flush();
            }
        });
        Self {
            endpoint,
            bodies,
            arrived,
            held,
            released,
            stop,
            worker: Some(worker),
        }
    }

    /// Holds the first answer until `release`, so the turn that sent it is
    /// still running.
    pub(super) fn hold_first(&self) {
        self.held.store(true, Ordering::Release);
    }

    pub(super) fn release(&self) {
        self.released.store(true, Ordering::Release);
    }

    /// How many requests have arrived so far.
    pub(super) fn arrived(&self) -> usize {
        self.arrived.load(Ordering::Acquire)
    }

    /// The request bodies as they arrived, in order.
    pub(super) fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }
}

impl Drop for Scripted {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).ok()?;
        if count == 0 {
            return None;
        }
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024, "bounded fixture request");
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let header = String::from_utf8_lossy(&bytes[..index]);
            let length = header.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })?;
            if bytes.len() >= index + 4 + length {
                return String::from_utf8(bytes[index + 4..index + 4 + length].to_vec()).ok();
            }
        }
    }
}

fn answer(text: &str) -> String {
    let delta = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": "stop"}]
    });
    format!(
        "data: {delta}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
    )
}
