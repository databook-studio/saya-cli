use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub struct Provider {
    endpoint: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

pub fn config(provider: &Provider) -> String {
    format!(
        "[ai]\nprovider = 'openai_compatible'\nbase_url = '{}'\nmodel = 'pty-fixture'\n\n[coding]\nenabled = true\n",
        provider.endpoint()
    )
}

impl Provider {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || serve(listener, stopped));
        Self {
            endpoint,
            stop,
            worker: Some(worker),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve(listener: TcpListener, stop: Arc<AtomicBool>) {
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
        let request = read_request(&mut stream);
        let body = if request.contains("UNTRUSTED CHILD TASK") {
            let actor = actor(&request);
            let turn = child_turns.entry(actor.clone()).or_default();
            let body = child_response(&actor, *turn);
            *turn += 1;
            body
        } else {
            let response = parent_response(parent_turn);
            parent_turn += 1;
            response
        };
        send(&mut stream, &body);
    }
}

fn parent_response(turn: usize) -> String {
    match turn {
        0 => tool(
            "delegate",
            json!({"tasks":[
                {"agent_id":"child-a","task":"write for child-a","write_paths":["a.txt"]},
                {"agent_id":"child-b","task":"write for child-b","write_paths":["b.txt"]}
            ]}),
        ),
        1 => tool("await_children", json!({})),
        _ => answer("native parent finished after joining children"),
    }
}

fn actor(request: &str) -> String {
    request
        .split("write for ")
        .nth(1)
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or("child")
        .to_owned()
}

fn child_response(actor: &str, turn: usize) -> String {
    if turn == 0 {
        tool(
            "coding_command_start",
            json!({"program":"sleep","argv":["20"],"timeout_seconds":30}),
        )
    } else {
        answer(&format!("{actor} child work complete"))
    }
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 4096];
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    loop {
        let count = match stream.read(&mut buffer) {
            Ok(0) => return String::new(),
            Ok(count) => count,
            Err(_) => return String::new(),
        };
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
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned();
            }
        }
    }
}

fn send(stream: &mut TcpStream, body: &str) {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    stream.flush().unwrap();
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
