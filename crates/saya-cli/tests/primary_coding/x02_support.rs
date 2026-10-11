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

pub const PROGRAM: &str = "sh";

pub struct Provider {
    pub endpoint: String,
    result: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Provider {
    pub fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let endpoint = format!("http://{}/v1", listener.local_addr().unwrap());
        let result = Arc::new(AtomicBool::new(false));
        let seen_result = Arc::clone(&result);
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let worker = thread::spawn(move || {
            let mut turns = BTreeMap::<bool, usize>::new();
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
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let body = read_request(&mut stream);
                let child = body.contains("UNTRUSTED CHILD TASK");
                if child && body.contains("safe MCP result") {
                    seen_result.store(true, Ordering::Release);
                }
                let turn = turns.entry(child).or_default();
                let response = response(child, *turn);
                *turn += 1;
                send(&mut stream, &response);
            }
        });
        Self {
            endpoint,
            result,
            stop,
            worker: Some(worker),
        }
    }

    pub fn saw_result(&self) -> bool {
        self.result.load(Ordering::Acquire)
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

pub fn profile(script: &str, effect: Option<&str>) -> String {
    let tools = effect.map_or_else(String::new, |effect| {
        format!("[coding.extensions.local.tools]\ninspect = '{effect}'\n")
    });
    format!(
        "[coding.extensions.local]\nprotocol = 'mcp'\nprogram = '{PROGRAM}'\nargv = ['-c', {}]\n{tools}",
        serde_json::to_string(script).unwrap()
    )
}

pub fn listed(name: &str) -> String {
    json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":name,"inputSchema":{"type":"object"},"annotations":{"readOnlyHint":true}}]}}).to_string()
}

pub fn invalid_schema() -> String {
    json!({"jsonrpc":"2.0","id":2,"result":{"tools":[{"name":"inspect","inputSchema":{"type":"array"}}]}}).to_string()
}

pub fn too_many_tools() -> String {
    let tools = (0..65)
        .map(|index| json!({"name":format!("tool-{index}"),"inputSchema":{"type":"object"}}))
        .collect::<Vec<_>>();
    json!({"jsonrpc":"2.0","id":2,"result":{"tools":tools}}).to_string()
}

pub fn paged() -> String {
    json!({"jsonrpc":"2.0","id":2,"result":{"tools":[],"nextCursor":"more"}}).to_string()
}

pub fn wrong_id() -> String {
    json!({"jsonrpc":"2.0","id":9,"result":{"tools":[]}}).to_string()
}

pub fn malformed() -> String {
    "{not-json}".into()
}

pub fn result() -> String {
    json!({"jsonrpc":"2.0","id":3,"result":{"content":[{"type":"text","text":"safe MCP result"}]}})
        .to_string()
}

pub fn server(listing: String, result: String) -> String {
    let initialize = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}});
    format!(
        "read init; printf '%s\\n' '{}'; read ready; read list; printf '%s\\n' '{}'; read call; printf called > mcp-called; printf '%s\\n' '{}'; read idle",
        initialize, listing, result
    )
}

pub fn server_with_oversized_result(listing: String) -> String {
    let initialize = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}});
    format!(
        "read init; printf '%s\\n' '{}'; read ready; read list; printf '%s\\n' '{}'; read call; printf called > mcp-called; printf '{{\"jsonrpc\":\"2.0\",\"id\":3,\"result\":\"'; i=0; while [ \"$i\" -lt 270000 ]; do printf x; i=$((i + 1)); done; printf '\"}}\\n'; read idle",
        initialize, listing
    )
}

pub fn server_with_oversized_schema() -> String {
    let initialize = json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18"}});
    format!(
        "read init; printf '%s\\n' '{}'; read ready; read list; printf '{{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{{\"tools\":[{{\"name\":\"inspect\",\"inputSchema\":{{\"type\":\"object\",\"description\":\"'; i=0; while [ \"$i\" -lt 17000 ]; do printf x; i=$((i + 1)); done; printf '\"}}}}]}}}}\\n'; read idle",
        initialize
    )
}

fn response(child: bool, turn: usize) -> String {
    match (child, turn) {
        (false, 0) => tool(
            "delegate",
            json!({"tasks":[{"agent_id":"mcp-child","task":"UNTRUSTED CHILD TASK: inspect","write_paths":[]}]}),
        ),
        (false, 1) => tool("await_children", json!({})),
        (false, _) | (true, _) if turn > 0 => answer("finished"),
        (true, 0) => tool(
            "coding_mcp",
            json!({"profile":"local","tool":"inspect","arguments":{}}),
        ),
        _ => answer("finished"),
    }
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

fn read_request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0, "provider request closed early");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 1024 * 1024);
        if let Some(header_end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..header_end]);
            let length = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|value| value.trim().parse::<usize>().unwrap())
                })
                .unwrap();
            if bytes.len() >= header_end + 4 + length {
                return String::from_utf8(bytes[header_end + 4..header_end + 4 + length].to_vec())
                    .unwrap();
            }
        }
    }
}

fn send(stream: &mut TcpStream, body: &str) {
    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
    stream.flush().unwrap();
}
