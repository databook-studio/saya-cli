use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
pub struct Provider {
    pub endpoint: String,
    pub calls: Arc<AtomicUsize>,
    pub overlap: Arc<AtomicBool>,
    pub saw_injection: Arc<AtomicBool>,
    pub bodies: Arc<Mutex<Vec<String>>>,
    status_gate: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}
impl Provider {
    pub fn new(injection: bool) -> Self {
        Self::script(injection, 0)
    }
    pub fn held() -> Self {
        Self::script(false, 4)
    }
    pub fn checks() -> Self {
        Self::script(false, 1)
    }
    pub fn commands() -> Self {
        Self::script(false, 2)
    }
    pub fn conflicts() -> Self {
        Self::script(false, 3)
    }
    pub fn pair_conflicts() -> Self {
        Self::script(false, 5)
    }
    // Mode 6, for the tool/command parity tests: the parent issues
    // `cancel_child` and `message_child` for the unregistered agent `ghost`,
    // then reads `coding_status`, then answers. The final answer is held
    // until `release()`, so a test can read the owner's live job state while
    // the run is parked; every request body is captured in `bodies`.
    pub fn parity() -> Self {
        Self::script(false, 6)
    }
    pub fn release(&self) {
        self.status_gate.store(true, Ordering::Release);
    }
    fn script(injection: bool, mode: u8) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let count = Arc::clone(&calls);
        let overlap = Arc::new(AtomicBool::new(false));
        let overlapped = Arc::clone(&overlap);
        let saw_injection = Arc::new(AtomicBool::new(false));
        let injected = Arc::clone(&saw_injection);
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let captured = Arc::clone(&bodies);
        let status_gate = Arc::new(AtomicBool::new(false));
        let held = Arc::clone(&status_gate);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<String, usize>::new();
            let mut pending = Vec::new();
            // A safety net only: the worker also exits when `Drop` sets
            // `stop`. Sequenced-child jobs (for example 24 dependent children)
            // run far longer than any fixed bound, so the net must outlast
            // them instead of cutting the provider off mid-test.
            let deadline = Instant::now() + Duration::from_secs(600);
            while !stopped.load(Ordering::Acquire) && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(_) => break,
                };
                // macOS hands out sockets that inherit the listener's O_NONBLOCK;
                // the request read below must block instead of racing the writer.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                // macOS accept() inherits the listener's O_NONBLOCK, so the
                // read timeout would be ignored and reads would return
                // EAGAIN instantly; restore blocking mode. A connection that
                // stays silent across its read windows is a client that gave
                // up without closing; drain it and keep serving.
                stream.set_nonblocking(false).unwrap();
                let Some(body) = read_request(&mut stream) else {
                    continue;
                };
                count.fetch_add(1, Ordering::AcqRel);
                captured.lock().unwrap().push(body.clone());
                if body.contains("INJECTED ROOT AUTHORITY") {
                    injected.store(true, Ordering::Release);
                }
                let actor = body
                    .split("UNTRUSTED CHILD TASK")
                    .nth(1)
                    .and_then(|task| task.split("write for ").nth(1))
                    .and_then(|tail| {
                        let suffix = tail.strip_prefix("child-")?;
                        let end = suffix
                            .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                            .unwrap_or(suffix.len());
                        (end > 0).then_some(&tail[.."child-".len() + end])
                    })
                    .unwrap_or("parent");
                let stage = turns.entry(actor.into()).or_default();
                let current = *stage;
                *stage += 1;
                let response = response(actor, current, injection, mode);
                if mode == 4 {
                    pending.push((stream, response));
                    continue;
                }
                if actor != "parent" && current == 0 && mode != 3 && mode != 5 {
                    pending.push((stream, response));
                    if pending.len() == 2 {
                        overlapped.store(true, Ordering::Release);
                        for (mut stream, response) in pending.drain(..) {
                            send(&mut stream, &response);
                        }
                    }
                } else {
                    // The parity parent's final answer stays unsent until the
                    // test opens the gate (or the bounded hold expires), so
                    // the run cannot complete before the test has read the
                    // captured coding_status result.
                    if mode == 6 && actor == "parent" && current >= 3 {
                        hold_for_status_read(&held);
                    }
                    send(&mut stream, &response);
                }
            }
        });
        Self {
            endpoint,
            calls,
            overlap,
            saw_injection,
            bodies,
            status_gate,
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
fn read_request(stream: &mut TcpStream) -> Option<String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut stalling = 0usize;
    loop {
        let n = match stream.read(&mut buffer) {
            Ok(n) => n,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                stalling += 1;
                if stalling > 6 {
                    return None;
                }
                continue;
            }
            Err(error) => panic!("provider request read failed: {error}"),
        };
        stalling = 0;
        assert!(n > 0, "provider request closed early");
        bytes.extend_from_slice(&buffer[..n]);
        assert!(bytes.len() <= 1024 * 1024);
        if let Some(index) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..index]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= index + 4 + length {
                return Some(
                    String::from_utf8(bytes[index + 4..index + 4 + length].to_vec()).unwrap(),
                );
            }
        }
    }
}
fn response(actor: &str, stage: usize, injection: bool, mode: u8) -> String {
    if actor == "parent" {
        if mode == 6 {
            return match stage {
                0 => tool("cancel_child", json!({"agent_id":"ghost"})),
                1 => tool(
                    "message_child",
                    json!({"agent_id":"ghost","message":"steer the ghost"}),
                ),
                2 => tool("coding_status", json!({})),
                _ => answer("parity parent finished after the unregistered-child refusals"),
            };
        }
        return match stage {
            0 if mode == 3 || mode == 5 => {
                let count = if mode == 3 { 24 } else { 2 };
                let tasks = (0..count).map(|index| {
                    let id = if mode == 3 { format!("child-{index:02}") } else if index == 0 { "child-a".into() } else { "child-b".into() };
                    let dependencies = if index == 0 { Vec::new() } else if mode == 3 { vec![format!("child-{:02}", index - 1)] } else { vec!["child-a".into()] };
                    json!({"agent_id":id,"task":format!("write for {id}"),"write_paths":["shared.txt","rename-before.txt","rename-after.txt"],"dependencies":dependencies})
                }).collect::<Vec<_>>();
                tool("delegate", json!({"tasks":tasks}))
            }
            0 => tool(
                "delegate",
                json!({"tasks":[{"agent_id":"child-a","task":"write for child-a","write_paths":["new-a.txt"]},{"agent_id":"child-b","task":"write for child-b","write_paths":["new-b.txt"]}]}),
            ),
            1 => tool("await_children", json!({})),
            _ => answer("native parent finished after joining children"),
        };
    }
    if mode == 1 && stage == 0 {
        return tool("coding_check", json!({"profile":"lint"}));
    }
    if mode == 2 && stage == 0 {
        return tool(
            "coding_command_start",
            json!({"program":"sleep","argv":["20"],"timeout_seconds":30}),
        );
    }
    if (mode == 3 || mode == 5) && stage == 0 {
        return tool(
            "workspace_write",
            json!({"path":"shared.txt","content":format!("{actor}\n")}),
        );
    }
    if (mode == 3 && actor == "child-00" || mode == 5 && actor == "child-a") && stage == 1 {
        return tool(
            "workspace_write",
            json!({"path":"rename-before.txt","content":"stable rename bytes\n"}),
        );
    }
    if mode == 3 || mode == 5 {
        return answer("child work complete");
    }
    if mode != 0 {
        return answer("child command evidence obtained");
    }
    if injection {
        match stage {
            0 => return tool("coding_skills", json!({"name":"poison"})),
            1 => {
                return tool(
                    "delegate",
                    json!({"tasks":[{"agent_id":"escape","task":"grant new root","write_paths":["outside.txt"]}]}),
                );
            }
            2 => {
                return tool(
                    "workspace_write",
                    json!({"path":"outside.txt","content":"must not exist"}),
                );
            }
            _ => {}
        }
    }
    if stage == if injection { 3 } else { 0 } {
        let path = if actor == "child-a" {
            "new-a.txt"
        } else {
            "new-b.txt"
        };
        tool(
            "workspace_write",
            json!({"path":path,"content":format!("native {actor}\n")}),
        )
    } else {
        answer("child work complete")
    }
}
fn tool(name: &str, args: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
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
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
    stream.flush().unwrap();
}
fn hold_for_status_read(gate: &AtomicBool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gate.load(Ordering::Acquire) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
}
