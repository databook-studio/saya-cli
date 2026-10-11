use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
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
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<String, usize>::new();
            let mut pending = Vec::new();
            let deadline = Instant::now() + Duration::from_secs(25);
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
                let body = read_request(&mut stream);
                count.fetch_add(1, Ordering::AcqRel);
                if body.contains("INJECTED ROOT AUTHORITY") {
                    injected.store(true, Ordering::Release);
                }
                let actor = body
                    .split("UNTRUSTED CHILD TASK")
                    .nth(1)
                    .and_then(|task| task.split("write for ").nth(1))
                    .map(|tail| {
                        let suffix = tail.strip_prefix("child-")?;
                        let end = suffix
                            .find(|ch: char| !ch.is_ascii_alphanumeric() && ch != '-')
                            .unwrap_or(suffix.len());
                        (end > 0).then_some(&tail[.."child-".len() + end])
                    })
                    .flatten()
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
                    send(&mut stream, &response);
                }
            }
        });
        Self {
            endpoint,
            calls,
            overlap,
            saw_injection,
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
fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let n = stream.read(&mut buffer).unwrap();
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
                return String::from_utf8(bytes[index + 4..index + 4 + length].to_vec()).unwrap();
            }
        }
    }
}
fn response(actor: &str, stage: usize, injection: bool, mode: u8) -> String {
    if actor == "parent" {
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
