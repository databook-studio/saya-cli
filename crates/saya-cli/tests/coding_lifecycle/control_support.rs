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
    time::Duration,
};

pub(crate) struct ControlProvider {
    pub(crate) endpoint: String,
    calls: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl ControlProvider {
    pub(crate) fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&calls);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || serve(listener, counted, stopped));
        Self {
            endpoint,
            calls,
            stop,
            worker: Some(worker),
        }
    }

    pub(crate) async fn wait_for_calls(&self, minimum: usize) {
        while self.calls.load(Ordering::Acquire) < minimum {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
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

fn serve(listener: TcpListener, calls: Arc<AtomicUsize>, stop: Arc<AtomicBool>) {
    let mut turns = BTreeMap::<String, usize>::new();
    while !stop.load(Ordering::Acquire) {
        let (mut stream, _) = match listener.accept() {
            Ok(stream) => stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
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
        send(&mut stream, &response(&actor, *turn));
        *turn += 1;
    }
}

fn actor(request: &str) -> String {
    request
        .split("UNTRUSTED CHILD TASK")
        .nth(1)
        .and_then(|task| task.split("write for ").nth(1))
        .and_then(|tail| tail.split_whitespace().next())
        .unwrap_or("child-a")
        .to_owned()
}

fn response(_actor: &str, turn: usize) -> String {
    if turn == 0 {
        return tool(
            "coding_command_start",
            json!({"program":"sleep","argv":["1"],"timeout_seconds":5}),
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
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    let mut stalling = 0usize;
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
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
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
}
