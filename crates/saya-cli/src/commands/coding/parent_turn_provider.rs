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

/// One request's scripted reply, in the order requests arrive: a `write` call
/// makes the turn change one file of the working checkout, a `read` or a search
/// call asks for one that changes nothing, and a text ends the turn that asked
/// for it. A request past the script is answered with text, as an unscripted
/// provider answers every request.
pub(crate) enum Reply {
    Text(&'static str),
    Write {
        path: &'static str,
        content: &'static str,
    },
    /// A `write` to a file named for the request that asked for it: a long
    /// scripted turn that makes progress on every call, with no repeated call.
    DistinctWrite,
    /// A `grep` for a pattern nothing matches, named for the request that asked
    /// for it: a settled call that moves nothing, and is never the same call
    /// twice, so the no-progress streak grows without tripping the repeat guard.
    FruitlessGrep,
    /// One call to a tool the test names — a plugin's tool, or any other name a
    /// scripted turn must reach with the arguments the test states.
    Call {
        name: &'static str,
        arguments: serde_json::Value,
    },
}

/// A local scripted provider: every request is one root turn, answered with a
/// final text. The first answer can be held, so a turn that is still running
/// is observable while it runs.
pub(crate) struct Scripted {
    pub(crate) endpoint: String,
    bodies: Arc<Mutex<Vec<String>>>,
    arrived: Arc<AtomicUsize>,
    held: Arc<AtomicBool>,
    /// The one request whose answer is held instead: the scripted helper's own
    /// request, so a helper that is working is observable while it works.
    held_at: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Scripted {
    /// A provider that answers every request with text.
    pub(crate) fn start() -> Self {
        Self::start_with_script(Vec::new())
    }

    /// The same, with the first `replies.len()` requests answered as scripted:
    /// one `write` call per turn, then the text that ends it.
    pub(crate) fn start_with_script(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let bodies: Arc<Mutex<Vec<String>>> = Arc::default();
        let arrived = Arc::new(AtomicUsize::new(0));
        let held = Arc::new(AtomicBool::new(false));
        let held_at = Arc::new(AtomicUsize::new(usize::MAX));
        let released = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (captured, seen, holding, holding_at, open, done) = (
            Arc::clone(&bodies),
            Arc::clone(&arrived),
            Arc::clone(&held),
            Arc::clone(&held_at),
            Arc::clone(&released),
            Arc::clone(&stop),
        );
        let script = Arc::new(replies);
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
                if (holding.load(Ordering::Acquire) && turn == 0)
                    || turn == holding_at.load(Ordering::Acquire)
                {
                    let deadline = Instant::now() + HOLD;
                    while !open.load(Ordering::Acquire) && Instant::now() < deadline {
                        thread::sleep(Duration::from_millis(5));
                    }
                }
                let response = match script.get(turn) {
                    Some(Reply::Text(text)) => answer(text),
                    Some(Reply::Write { path, content }) => tool_call(
                        &format!("call-{turn}"),
                        "write",
                        &serde_json::json!({"path": path, "content": content}),
                    ),
                    Some(Reply::DistinctWrite) => tool_call(
                        &format!("call-{turn}"),
                        "write",
                        &serde_json::json!({
                            "path": format!("note-{turn}.txt"),
                            "content": format!("note {turn}\n"),
                        }),
                    ),
                    Some(Reply::FruitlessGrep) => tool_call(
                        &format!("call-{turn}"),
                        "grep",
                        &serde_json::json!({"pattern": format!("nothing-matches-{turn}")}),
                    ),
                    Some(Reply::Call { name, arguments }) => {
                        tool_call(&format!("call-{turn}"), name, arguments)
                    }
                    None => answer(&format!("turn {turn} answered")),
                };
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
            held_at,
            released,
            stop,
            worker: Some(worker),
        }
    }

    /// Holds the answer to `request` — the request at that index — until
    /// `release`, so the loop that sent it is still running: a scripted
    /// helper's own request is the second one.
    pub(crate) fn hold_request(&self, request: usize) {
        self.held_at.store(request, Ordering::Release);
    }

    /// Holds the first answer until `release`, so the turn that sent it is
    /// still running.
    pub(crate) fn hold_first(&self) {
        self.held.store(true, Ordering::Release);
    }

    pub(crate) fn release(&self) {
        self.released.store(true, Ordering::Release);
    }

    /// How many requests have arrived so far.
    pub(crate) fn arrived(&self) -> usize {
        self.arrived.load(Ordering::Acquire)
    }

    /// The request bodies as they arrived, in order.
    pub(crate) fn bodies(&self) -> Vec<String> {
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

/// One tool call as the OpenAI-compatible stream carries it: the arguments are
/// the serialized object the model sent.
fn tool_call(id: &str, name: &str, arguments: &serde_json::Value) -> String {
    let delta = serde_json::json!({
        "choices": [{"index": 0, "delta": {"tool_calls": [{"index": 0, "id": id, "type": "function",
            "function": {"name": name, "arguments": arguments.to_string()}}]},
            "finish_reason": "tool_calls"}]
    });
    format!(
        "data: {delta}\n\ndata: {}\n\ndata: [DONE]\n\n",
        serde_json::json!({"choices": [], "usage": {"prompt_tokens": 10, "completion_tokens": 5}})
    )
}
