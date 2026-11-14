use super::DEADLINE;
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Provider {
    endpoint: String,
    child_actors: Arc<Mutex<BTreeSet<String>>>,
    finished_actors: Arc<Mutex<BTreeSet<String>>>,
    child_a_answer_released: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

/// The read-loop pace keeps the provider's 80-request bound from expiring
/// during the PTY's control window; final answers are not paced. The bound is a
/// product cap the fixture cannot raise and a live helper spends one request
/// per turn, so the pace is scaled with the window, which is itself sized for a
/// loaded machine: 80 turns at this pace outlast the whole control window, not
/// just its unloaded length. A helper's first two turns are not paced, so
/// nothing the test waits on is delayed by it.
const READ_LOOP_PACE: Duration = Duration::from_secs(1);

pub fn config(provider: &Provider) -> String {
    // The default root mode: a Git working tree runs in place, and its helpers
    // share the root's folder.
    format!(
        "[ai]\nprovider = 'openai_compatible'\nbase_url = '{}'\nmodel = 'pty-fixture'\n",
        provider.endpoint()
    )
}

impl Provider {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let child_actors = Arc::new(Mutex::new(BTreeSet::new()));
        let finished_actors = Arc::new(Mutex::new(BTreeSet::new()));
        let child_a_answer_released = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let actors = Arc::clone(&child_actors);
        let finished = Arc::clone(&finished_actors);
        let answer_released = Arc::clone(&child_a_answer_released);
        let worker =
            thread::spawn(move || serve(listener, stopped, actors, finished, answer_released));
        Self {
            endpoint,
            child_actors,
            finished_actors,
            child_a_answer_released,
            stop,
            worker: Some(worker),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    pub fn wait_for_child(&self, actor: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.child_actors.lock().unwrap().contains(actor) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("provider did not receive a request for {actor}");
    }

    pub fn wait_for_child_finished(&self, actor: &str, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if self.finished_actors.lock().unwrap().contains(actor) {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("provider did not receive a finishing turn for {actor}");
    }

    pub fn release_child_a_answer(&self) {
        self.child_a_answer_released.store(true, Ordering::Release);
    }

    pub fn shutdown(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !worker.is_finished() && Instant::now() < deadline {
                thread::sleep(Duration::from_millis(10));
            }
            if !worker.is_finished() {
                self.worker = Some(worker);
                panic!("provider worker did not finish within its read deadline");
            }
            worker
                .join()
                .expect("provider worker exits without panicking");
        }
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take();
    }
}

fn serve(
    listener: TcpListener,
    stop: Arc<AtomicBool>,
    child_actors: Arc<Mutex<BTreeSet<String>>>,
    finished_actors: Arc<Mutex<BTreeSet<String>>>,
    child_a_answer_released: Arc<AtomicBool>,
) {
    let deadline = Instant::now() + DEADLINE;
    let mut parent_turn = 0;
    let mut child_turns = BTreeMap::<String, usize>::new();
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => break,
        };
        // macOS hands out sockets that inherit the listener's O_NONBLOCK; the
        // request read below must block instead of racing the writer.
        stream.set_nonblocking(false).unwrap();
        let request = match read_request(&mut stream) {
            Ok(Some(request)) => request,
            Ok(None) => continue,
            Err(error) if peer_disconnected(&error) => continue,
            Err(error) => panic!("provider request read failed: {error}"),
        };
        let mut finished_actor = None;
        let body = if request.contains("UNTRUSTED CHILD TASK") {
            let actor = actor(&request);
            child_actors.lock().unwrap().insert(actor.clone());
            let turn = child_turns.entry(actor.clone()).or_default();
            let allow_answer = child_a_answer_released.load(Ordering::Acquire);
            let finishing = actor == "child-a" && *turn >= 2 && allow_answer;
            let paced = *turn >= 2 && !finishing;
            let body = child_response(&actor, *turn, allow_answer);
            *turn += 1;
            if finishing {
                finished_actor = Some(actor);
            }
            if paced {
                // Leave request budget for the operator's real controls; the
                // listener never waits for the controller to release answers.
                thread::sleep(READ_LOOP_PACE);
            }
            body
        } else {
            let response = parent_response(parent_turn);
            parent_turn += 1;
            response
        };
        if let Err(error) = send(&mut stream, &body) {
            if peer_disconnected(&error) {
                continue;
            }
            panic!("provider response write failed: {error}");
        }
        if let Some(actor) = finished_actor {
            finished_actors.lock().unwrap().insert(actor);
        }
    }
}

/// One `helpers` call is one helper and the call waits for it, so the two
/// children run one after the other: child-a first, then child-b.
fn parent_response(turn: usize) -> String {
    match turn {
        0 => helpers("call-task-a", "write for child-a", "child-a"),
        1 => helpers("call-task-b", "write for child-b", "child-b"),
        _ => answer("native parent finished after its helpers"),
    }
}

fn helpers(id: &str, description: &str, agent: &str) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":id,"type":"function","function":{"name":"helpers","arguments":json!({"description":description,"prompt":format!("write for {agent}"),"subagent_type":"general","agent_id":agent}).to_string()}}]},"finish_reason":"tool_calls"}]}),
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}

fn actor(request: &str) -> String {
    request
        .split("write for ")
        .nth(1)
        .map(|tail| {
            // The body quotes the delegated task in several serializations
            // (`"task":"write for child-a"}`, history echoes, context
            // blocks); only the agent id itself is stable — stop at the
            // first character that cannot belong to an id, never at
            // whitespace, or the JSON that follows leaks into the key.
            tail.chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect()
        })
        .unwrap_or_else(|| "child".to_owned())
}

/// Every turn reads a file the working checkout already holds, which keeps the
/// helper live across the operator's control calls. child-a answers only after
/// the test releases it following the real control receipts; child-b stays live
/// until the test cancels it.
fn child_response(actor: &str, turn: usize, allow_child_a_answer: bool) -> String {
    if actor == "child-a" && turn >= 2 && allow_child_a_answer {
        answer(&format!("{actor} child work complete"))
    } else {
        tool("read", json!({"path":"baseline.txt"}))
    }
}

fn read_request(stream: &mut TcpStream) -> std::io::Result<Option<String>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    // Load ceiling, not a product bound: a starved client can take a while to
    // flush one request body, and a timeout here fails the fixture rather than
    // the behavior under test.
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(None);
        }
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
            if length <= 128 * 1024 && bytes.len() >= head + 4 + length {
                return Ok(Some(
                    String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned(),
                ));
            }
        }
    }
}

fn send(stream: &mut TcpStream, body: &str) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )?;
    stream.flush()
}

fn peer_disconnected(error: &std::io::Error) -> bool {
    matches!(
        error.kind(),
        std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
    )
}

fn tool(name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
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
