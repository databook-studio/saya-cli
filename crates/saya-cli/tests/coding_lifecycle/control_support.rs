//! Scripted provider for the control-surface tests (`controls`, `root_cancel`).
//!
//! The children are *held*, not timed. A child's first response waits in
//! `pending` until the test calls `release()`, so a test can read the job's
//! live state and drive its controls while the children are provably parked on
//! their first request — no race against a sleep. Past its first turn only
//! `child-a` is ever answered, so `child-b` stays live until the test cancels
//! it.
//!
//! A test that must cancel `child-b` uses [`ControlProvider::holding_child_b`]:
//! every `child-b` response, its first turn included, then waits for
//! `release_child_b()` instead of `release()`. The test opens that gate only
//! after the cancel is acknowledged, so `child-b` cannot reach an answer and
//! finish before the cancel lands.

use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) struct ControlProvider {
    pub(crate) endpoint: String,
    calls: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
    child_b_released: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ControlProvider {
    pub(crate) fn new() -> Self {
        Self::with_child_b_gate(false)
    }

    /// A provider whose `child-b` responses stay held until `release_child_b()`,
    /// whatever `release()` does. A test cancels `child-b` while it is parked
    /// on its first request and opens the gate only once the cancel is
    /// acknowledged, so the child can never be answered first.
    pub(crate) fn holding_child_b() -> Self {
        Self::with_child_b_gate(true)
    }

    fn with_child_b_gate(child_b_held: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let released = Arc::new(AtomicBool::new(false));
        let open = Arc::clone(&released);
        let child_b_held = Arc::new(AtomicBool::new(child_b_held));
        let held = Arc::clone(&child_b_held);
        let child_b_released = Arc::new(AtomicBool::new(false));
        let opened = Arc::clone(&child_b_released);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || serve(listener, counted, open, held, opened, stopped));
        Self {
            endpoint,
            calls,
            released,
            child_b_released,
            stop,
            worker: Some(worker),
        }
    }

    pub(crate) async fn wait_for_calls(&self, minimum: usize) {
        while self.calls.load(Ordering::Acquire) < minimum {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Lets the held first turns run. Called once the test has read the live
    /// job state and issued the controls it wants acknowledged at the
    /// children's next turn boundary. A `child-b` gate opened by
    /// [`Self::holding_child_b`] is unaffected.
    pub(crate) fn release(&self) {
        self.released.store(true, Ordering::Release);
    }

    /// Opens the `child-b` gate. Called after the cancel is acknowledged; the
    /// cancelled child may already have closed its request, so the fixture
    /// ignores a write to that closed connection.
    pub(crate) fn release_child_b(&self) {
        self.child_b_released.store(true, Ordering::Release);
    }
}

impl Drop for ControlProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

/// Why a response is parked, and therefore which signal opens it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Hold {
    /// The child's first turn: `release()` opens it.
    FirstTurn,
    /// Every `child-b` response while its gate is held, and every later turn
    /// of a child other than `child-a`: only `release_child_b()` opens it.
    ChildB,
}

fn hold(actor: &str, turn: usize, released: bool, child_b_held: bool) -> Option<Hold> {
    if child_b_held && actor == "child-b" {
        return Some(Hold::ChildB);
    }
    if turn == 0 && !released {
        return Some(Hold::FirstTurn);
    }
    (actor != "child-a" && turn >= 1).then_some(Hold::ChildB)
}

struct Pending {
    stream: TcpStream,
    body: String,
    hold: Hold,
}

fn serve(
    listener: TcpListener,
    calls: Arc<AtomicUsize>,
    released: Arc<AtomicBool>,
    child_b_held: Arc<AtomicBool>,
    child_b_released: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
) {
    let mut turns = BTreeMap::<String, usize>::new();
    let mut pending = Vec::<Pending>::new();
    while !stop.load(Ordering::Acquire) {
        let (mut stream, _) = match listener.accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if released.load(Ordering::Acquire) {
                    flush(&mut pending, Hold::FirstTurn);
                }
                if child_b_released.load(Ordering::Acquire) {
                    flush(&mut pending, Hold::ChildB);
                }
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => return,
        };
        let Some(request) = read_request(&mut stream) else {
            continue;
        };
        calls.fetch_add(1, Ordering::AcqRel);
        let actor = actor(&request);
        let turn = turns.entry(actor.clone()).or_default();
        let current = *turn;
        *turn += 1;
        let body = response(&actor, current);
        // A child's first turn waits for `release()`, and no child but
        // `child-a` is ever answered past it: the parked children stay live
        // (and their pause requests stay unacknowledged) until the test is
        // done reading the job's state and has opened the gate. A gated
        // `child-b` waits for its own signal even on its first turn.
        let held = hold(
            &actor,
            current,
            released.load(Ordering::Acquire),
            child_b_held.load(Ordering::Acquire),
        );
        if let Some(hold) = held {
            pending.push(Pending { stream, body, hold });
            continue;
        }
        send(&mut stream, &body);
    }
}

/// Answers every parked request whose hold `open` names, leaving the rest
/// parked. A parked child may have been cancelled and closed its connection,
/// so a failed write here is expected and ignored.
fn flush(pending: &mut Vec<Pending>, open: Hold) {
    pending.retain_mut(|entry| {
        if entry.hold != open {
            return true;
        }
        let _ = try_send(&mut entry.stream, &entry.body);
        false
    });
}

fn actor(request: &str) -> String {
    // The request body is JSON, so the assignment's newlines arrive escaped
    // (`\n` is two characters). Stop at the first character that is not part
    // of the agent name instead of splitting on whitespace, which would keep
    // the escaped tail and make every actor compare unequal to `child-a`.
    request
        .split("UNTRUSTED CHILD TASK")
        .nth(1)
        .and_then(|task| task.split("write for ").nth(1))
        .and_then(|tail| {
            let end = tail
                .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                .unwrap_or(tail.len());
            (end > 0).then(|| tail[..end].to_owned())
        })
        .unwrap_or_else(|| "child-a".to_owned())
}

fn response(_actor: &str, turn: usize) -> String {
    if turn == 0 {
        // A command is one synchronous `shell` call; the sandbox keeps a
        // headless job from asking for host approval.
        return tool(
            "shell",
            json!({"command":"sleep 1","description":"run the command","sandbox":true}),
        );
    }
    answer("child work complete")
}

fn read_request(stream: &mut TcpStream) -> Option<String> {
    // macOS accept() inherits the listener's O_NONBLOCK, so reads would
    // return EAGAIN instantly; restore blocking mode. A connection that
    // stays silent across its read windows is a client that gave up without
    // closing; drain it and keep serving.
    stream.set_nonblocking(false).unwrap();
    // Load ceiling, not a product bound: a starved client can take a while to
    // flush one request body, and the six silent windows below are what decides
    // that a connection was abandoned.
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut stalling = 0usize;
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                stalling += 1;
                if stalling > 6 {
                    return None;
                }
                continue;
            }
            Err(error) => panic!("provider request read failed: {error}"),
        };
        stalling = 0;
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            });
            if let Some(length) = length.filter(|length| bytes.len() >= end + 4 + length) {
                return Some(String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap());
            }
        }
    }
}

fn tool(name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"control","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

fn answer(text: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"content":text},"finish_reason":"stop"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

fn send(stream: &mut TcpStream, body: &str) {
    try_send(stream, body).unwrap();
}

fn try_send(stream: &mut TcpStream, body: &str) -> io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}
