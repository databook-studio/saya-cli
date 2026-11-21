//! The provider a compaction test drives: a local scripted one whose usage says
//! how full the run's context was.
//!
//! It is the scripted provider's shape with the one difference that is the whole
//! point here: the usage it reports states the context the request measured, so a
//! test can put a settled turn at exactly 96% — or 94% — of a known window. It
//! answers the summarising call with the summary text the test states, so a test
//! can also ask it for a summary that cannot be applied (an empty one) and watch
//! the trigger latch.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// The phrase every summarising request carries: the engine's own prompt. A
/// request that carries it is a compaction; every other one is a turn.
pub(super) const SUMMARISE: &str = "Summarise the following earlier conversation turns";

/// The window the tests' model is known to have: `gpt-4o`'s.
pub(super) const WINDOW: u64 = 128_000;

/// The answering report of a turn at 96% of [`WINDOW`].
pub(super) const AT_96: u64 = 122_880;

/// The answering report of a turn at 94% of [`WINDOW`].
pub(super) const AT_94: u64 = 120_320;

/// The model the tests state: one the window table knows.
pub(super) const KNOWN_MODEL: &str = "gpt-4o";

/// A local provider: every request is answered as a stream — a turn with text,
/// the summarising call with the summary the test stated — and every answer
/// reports `input_tokens` as the context it measured.
pub(super) struct Reporting {
    pub(super) endpoint: String,
    bodies: Arc<Mutex<Vec<String>>>,
    summarised: Arc<AtomicUsize>,
    summary: Arc<Mutex<String>>,
    input_tokens: u64,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Reporting {
    /// A provider reporting `input_tokens` on every answer, and answering the
    /// summarising call with `summary`.
    pub(super) fn start(input_tokens: u64, summary: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("the fixture provider binds");
        listener
            .set_nonblocking(true)
            .expect("the fixture provider polls");
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies: Arc<Mutex<Vec<String>>> = Arc::default();
        let summarised = Arc::new(AtomicUsize::new(0));
        let summary: Arc<Mutex<String>> = Arc::new(Mutex::new(summary.to_owned()));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, seen, said, done) = (
            Arc::clone(&bodies),
            Arc::clone(&summarised),
            Arc::clone(&summary),
            Arc::clone(&stop),
        );
        let worker = thread::spawn(move || {
            let mut turns = 0usize;
            while !done.load(Ordering::Acquire) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                stream
                    .set_nonblocking(false)
                    .expect("the fixture provider accepts a request");
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .expect("the fixture provider bounds a read");
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                captured.lock().unwrap().push(body.clone());
                let text = if body.contains(SUMMARISE) {
                    seen.fetch_add(1, Ordering::AcqRel);
                    said.lock().unwrap().clone()
                } else {
                    turns += 1;
                    format!("turn {} answered", turns - 1)
                };
                let answer = answer(&text, input_tokens);
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
                let _ = stream.flush();
            }
        });
        Self {
            endpoint,
            bodies,
            summarised,
            summary,
            input_tokens,
            stop,
            worker: Some(worker),
        }
    }

    /// Answers the summarising call with `summary` from now on: a test that
    /// latched the trigger on a failure makes the next try succeed.
    pub(super) fn set_summary(&self, summary: &str) {
        *self.summary.lock().unwrap() = summary.to_owned();
    }

    /// How many summarising calls have arrived: exactly one per compaction.
    pub(super) fn summarised(&self) -> usize {
        self.summarised.load(Ordering::Acquire)
    }

    /// The request the provider received last.
    pub(super) fn last_body(&self) -> String {
        self.bodies
            .lock()
            .unwrap()
            .pop()
            .expect("the provider received a request")
    }

    /// The request bodies as they arrived, in order.
    pub(super) fn bodies(&self) -> Vec<String> {
        self.bodies.lock().unwrap().clone()
    }

    /// The context every answer of this provider reports.
    pub(super) fn input_tokens(&self) -> u64 {
        self.input_tokens
    }

    /// Waits until `count` requests have arrived.
    pub(super) fn await_bodies(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while self.bodies().len() < count {
            assert!(
                Instant::now() < deadline,
                "the provider must receive {count} requests"
            );
            thread::sleep(Duration::from_millis(5));
        }
    }
}

impl Drop for Reporting {
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

/// One streamed answer carrying `text`, reporting `input_tokens` as the context
/// the request measured.
fn answer(text: &str, input_tokens: u64) -> String {
    let delta = serde_json::json!({
        "choices": [{"index": 0, "delta": {"content": text}, "finish_reason": "stop"}]
    });
    format!(
        "data: {delta}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({
            "choices": [],
            "usage": {"prompt_tokens": input_tokens, "completion_tokens": 5}
        })
    )
}
