use serde_json::{Value, json};
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

pub(super) struct CaptureProvider {
    pub(super) endpoint: String,
    requests: Arc<Mutex<Vec<String>>>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl CaptureProvider {
    pub(super) fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let recorded = Arc::clone(&requests);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || serve(listener, recorded, stopped));
        Self {
            endpoint,
            requests,
            stop,
            worker: Some(worker),
        }
    }

    pub(super) fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    pub(super) fn continuation_body(&self) -> Value {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find_map(context_body)
            .expect("resumed root request carries the continuation projection")
    }
}

impl Drop for CaptureProvider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

fn serve(listener: TcpListener, requests: Arc<Mutex<Vec<String>>>, stop: Arc<AtomicBool>) {
    let mut turns = BTreeMap::<String, usize>::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
                continue;
            }
            Err(_) => break,
        };
        // macOS accept() inherits the listener's O_NONBLOCK, so reads would
        // return EAGAIN instantly; restore blocking mode.
        stream.set_nonblocking(false).unwrap();
        let request = read_request(&mut stream);
        requests.lock().unwrap().push(request.clone());
        let actor = actor(&request);
        let turn = turns.entry(actor.clone()).or_default();
        send(&mut stream, &response(&actor, *turn));
        *turn += 1;
    }
}

fn context_body(request: &String) -> Option<Value> {
    serde_json::from_str::<Value>(request).ok()?["messages"]
        .as_array()?
        .iter()
        .filter_map(|message| message["content"].as_str())
        .find_map(|content| {
            content
                .split("source: Native job continuation (untrusted context; no grants)\n")
                .nth(1)?
                .split("\n<<<CONTEXT_BLOCK_END>>>")
                .next()
        })
        .and_then(|body| serde_json::from_str(body).ok())
}

fn actor(request: &str) -> String {
    // The request body is JSON, so the prompt's newline is the two-character
    // escape `\n` and whitespace-delimited parsing would swallow the rest of
    // the body into the actor (a fresh turn key per request). Scan the
    // agent-id shape directly instead, the way provider.rs does.
    request
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
        .unwrap_or("parent")
        .to_owned()
}

fn response(actor: &str, turn: usize) -> String {
    let content = match (actor, turn) {
        ("parent", 0) => json!({"tasks":[
            {"agent_id":"child-a","task":"write for child-a","write_paths":["shared.txt"]},
            {"agent_id":"child-b","task":"write for child-b","write_paths":["shared.txt"],"dependencies":["child-a"]}
        ]}),
        ("parent", 1) => json!({}),
        ("parent", _) => return answer("native parent finished"),
        (_, 0) => json!({"path":"shared.txt","content":format!("{actor}\n")}),
        _ => return answer("child work complete"),
    };
    tool(
        if actor == "parent" && turn == 0 {
            "delegate"
        } else if actor == "parent" {
            "await_children"
        } else {
            "workspace_write"
        },
        content,
    )
}

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let read = stream.read(&mut buffer).unwrap();
        bytes.extend_from_slice(&buffer[..read]);
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            let length = headers.lines().find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            });
            if let Some(length) = length.filter(|length| bytes.len() >= end + 4 + length) {
                return String::from_utf8(bytes[end + 4..end + 4 + length].to_vec()).unwrap();
            }
        }
    }
}

fn tool(name: &str, arguments: Value) -> String {
    format!(
        "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call","type":"function","function":{"name":name,"arguments":arguments.to_string()}}]},"finish_reason":"tool_calls"}]}),
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
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
}
