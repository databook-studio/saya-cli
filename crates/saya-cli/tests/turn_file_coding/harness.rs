//! The scripted multi-round mock provider and scratch environment for the
//! turn-file coding tests: one scripted SSE reply per accepted request, each
//! round computed from the request body it answers, every raw request
//! captured for assertions.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// One scripted provider reply: a tool call or the final text.
pub enum Round {
    Tool {
        name: &'static str,
        arguments: serde_json::Value,
    },
    Text(&'static str),
}

/// A round is computed from the request body it answers — round N's body
/// already carries round N-1's tool results (the read's `digest` for round 2).
pub type RoundFn = Box<dyn FnMut(&str) -> Round + Send>;

pub struct Scripted {
    pub address: String,
    /// Every raw HTTP request the provider received, in order.
    pub bodies: Arc<Mutex<Vec<String>>>,
}

fn sse(round: &Round, index: usize) -> String {
    let chunk = match round {
        Round::Tool { name, arguments } => serde_json::json!({
            "choices": [{"delta": {"tool_calls": [{
                "index": 0,
                "id": format!("call_{index}"),
                "function": {"name": name, "arguments": arguments.to_string()}
            }]}}]
        }),
        Round::Text(text) => serde_json::json!({"choices": [{"delta": {"content": text}}]}),
    };
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

/// Serves one scripted reply per accepted request (pattern of
/// tests/chart_cleanup.rs), then stops. Extra requests are not answered.
pub fn script(rounds: Vec<RoundFn>) -> Scripted {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = Arc::clone(&bodies);
    thread::spawn(move || {
        for (index, mut round) in rounds.into_iter().enumerate() {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut raw = Vec::new();
            let mut buf = [0_u8; 8192];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(size) => raw.extend_from_slice(&buf[..size]),
                }
            }
            let request = String::from_utf8_lossy(&raw).into_owned();
            seen.lock().unwrap().push(request.clone());
            let body = sse(&round(&request), index);
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
    });
    Scripted { address, bodies }
}

pub struct Env {
    pub root: PathBuf,
    pub project: PathBuf,
    pub sessions: PathBuf,
}

pub fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("saya-turn-coding-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let project = root.join("project");
    let sessions = root.join("sessions");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join(".git")).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    Env {
        root,
        project,
        sessions,
    }
}

/// `saya --workspace <project> --turn-file <turn> --approval-mode <mode> --format ndjson`
/// with NO connections file and NO profile.
pub fn run(env: &Env, scripted: &Scripted, mode: &str, turn: &str) -> std::process::Output {
    let turn_file = env.root.join("turn.txt");
    std::fs::write(&turn_file, turn).unwrap();
    Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--workspace",
            env.project.to_str().unwrap(),
            "--turn-file",
            turn_file.to_str().unwrap(),
            "--approval-mode",
            mode,
            "--allow-data-sharing",
            "--format",
            "ndjson",
        ])
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", scripted.address))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

/// The parsed request body of one captured raw request.
pub fn request_json(raw: &str) -> serde_json::Value {
    let (_, body) = raw.split_once("\r\n\r\n").expect("HTTP request has a body");
    serde_json::from_str(body).expect("request body parses as JSON")
}

/// The tool names the request advertises (`tools[].function.name`).
pub fn tool_names(request: &serde_json::Value) -> Vec<&str> {
    request["tools"]
        .as_array()
        .expect("request advertises tools")
        .iter()
        .map(|tool| {
            tool["function"]["name"]
                .as_str()
                .expect("tool names its function")
        })
        .collect()
}

/// The model-facing tool results in the request, in conversation order.
pub fn tool_results(request: &serde_json::Value) -> Vec<&str> {
    request["messages"]
        .as_array()
        .expect("request carries messages")
        .iter()
        .filter(|message| message["role"] == "tool")
        .filter_map(|message| message["content"].as_str())
        .collect()
}

/// The `digest` the read tool reported: parsed from the tool-result message
/// in the request body — the read's JSON result carries `digest` (a plain
/// string scan cannot work: the tool descriptions themselves say "digest").
pub fn digest_from(raw: &str) -> String {
    for content in tool_results(&request_json(raw)) {
        if let Some(at) = content.find("\"digest\":\"") {
            let hex: String = content[at + 10..]
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if hex.len() == 64 {
                return hex;
            }
        }
    }
    panic!(
        "no tool result in the request carries a digest: {:?}",
        tool_results(&request_json(raw))
    );
}

pub fn stdout(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
