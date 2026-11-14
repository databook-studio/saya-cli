//! A self-contained scripted provider for the capability journeys: the same
//! OpenAI-compatible SSE transport the other native fixtures speak, with the
//! scripts owned here so the shared lifecycle fixtures stay untouched. Root
//! and helper turns share one transport; a helper turn is the request whose
//! task block names it.

use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[path = "provider_script.rs"]
pub(super) mod script;

pub struct Provider {
    pub endpoint: String,
    pub bodies: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Provider {
    /// Two database probe helpers, then the root's own database reads.
    pub fn database() -> Self {
        Self::script(10)
    }
    /// The root inspects its own working checkout, then writes its change.
    pub fn inspect_review() -> Self {
        Self::script(11)
    }
    /// One helper writes the tracked file; the owner assembles and reviews it.
    pub fn integrated_review() -> Self {
        Self::script(12)
    }

    pub(crate) fn script(mode: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bodies);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<String, usize>::new();
            let deadline = Instant::now() + Duration::from_secs(120);
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                // macOS hands out sockets that inherit the listener's
                // O_NONBLOCK; the request read below must block instead of
                // racing the writer.
                stream.set_nonblocking(false).unwrap();
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                let actor = actor_of(&body);
                captured.lock().unwrap().push(body.clone());
                let stage = turns.entry(actor.clone()).or_default();
                let current = *stage;
                *stage += 1;
                let response = script::response(&actor, current, mode, &body);
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
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

/// The agent id a helper turn belongs to: the child task text carries
/// `write for <agent>`, which is how the scripts name their helpers.
fn actor_of(body: &str) -> String {
    body.split("UNTRUSTED CHILD TASK")
        .nth(1)
        .and_then(|task| task.split("write for ").nth(1))
        .map(|tail| {
            tail.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect()
        })
        .filter(|actor: &String| !actor.is_empty())
        .unwrap_or_else(|| "parent".to_owned())
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    let mut stalling = 0usize;
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(count) => count,
            // The read timeout surfaces as EAGAIN/WouldBlock on macOS while the
            // request is still arriving; tolerate a few empty windows, then
            // treat the connection as abandoned.
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                stalling += 1;
                if stalling > 6 {
                    return None;
                }
                continue;
            }
            Err(_) => return None,
        };
        if count == 0 {
            return None;
        }
        stalling = 0;
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024, "provider request is bounded");
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..head]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|value| value.trim().parse::<usize>().ok())
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + length {
                return Some(
                    String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned(),
                );
            }
        }
    }
}

pub(super) fn sse(value: Value) -> String {
    format!(
        "data: {value}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
