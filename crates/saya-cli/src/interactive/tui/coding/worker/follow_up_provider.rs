use serde_json::json;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};
pub(crate) struct Provider {
    pub endpoint: String,
    pub started: Arc<AtomicBool>,
    pub release: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Provider {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let started = Arc::new(AtomicBool::new(false));
        let release = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let (seen, gate, done) = (started.clone(), release.clone(), stop.clone());
        let worker = thread::spawn(move || {
            let (mut parents, mut children) = (0, 0);
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
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let Some(request) = read_request(&mut stream) else {
                    continue;
                };
                let response = if request.contains("UNTRUSTED CHILD TASK") {
                    children += 1;
                    if children == 2 {
                        seen.store(true, Ordering::Release);
                        while !gate.load(Ordering::Acquire) && !done.load(Ordering::Acquire) {
                            thread::sleep(Duration::from_millis(5));
                        }
                    }
                    answer()
                } else {
                    let stage = parents;
                    parents += 1;
                    match stage {
                        0 => tool(
                            "helpers",
                            json!({"description":"bounded initial child","prompt":"bounded initial child","subagent_type":"general","agent_id":"worker-child","write_paths":["tracked.txt"]}),
                        ),
                        _ => answer(),
                    }
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
            started,
            release,
            stop,
            worker: Some(worker),
        }
    }
}
impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.release.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
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
        assert!(bytes.len() <= 1024 * 1024);
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
fn tool(name: &str, args: serde_json::Value) -> String {
    sse(
        json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":format!("call-{name}"),"type":"function","function":{"name":name,"arguments":args.to_string()}}]},"finish_reason":"tool_calls"}]}),
    )
}
fn answer() -> String {
    sse(
        json!({"choices":[{"index":0,"delta":{"content":"bounded phase complete"},"finish_reason":"stop"}]}),
    )
}
fn sse(value: serde_json::Value) -> String {
    format!(
        "data: {value}\n\ndata: {}\n\ndata: [DONE]\n\n",
        json!({"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5}})
    )
}
