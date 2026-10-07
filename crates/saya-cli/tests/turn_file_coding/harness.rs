//! The scripted multi-round mock provider and scratch environment for the
//! turn-file coding tests: one scripted SSE reply per accepted request, each
//! round computed from the request body it answers, every raw request
//! captured for assertions.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

/// One scripted provider reply: a tool call or the final text.
pub enum Round {
    Tool {
        name: &'static str,
        arguments: serde_json::Value,
    },
    Text(&'static str),
    /// A held model stream: headers WITHOUT a content length, one content
    /// chunk (the marker), flush, signal, then block until the client goes
    /// away — the pattern of tests/mvp/session.rs, so the test can cancel a
    /// mid-request child deterministically (never by sleeping).
    Hold(&'static str),
    /// Every attempt answers HTTP 500: the provider path that fails a turn.
    Fail500,
}

/// A round is computed from the request body it answers — round N's body
/// already carries round N-1's tool results (the read's `digest` for round 2).
pub type RoundFn = Box<dyn FnMut(&str) -> Round + Send>;

pub struct Scripted {
    pub address: String,
    /// Every raw HTTP request the provider received, in order.
    pub bodies: Arc<Mutex<Vec<String>>>,
    /// Set once a hold round has flushed its first chunk: the client is
    /// mid-request and the test may cancel.
    pub held: Arc<AtomicBool>,
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
        // The hold and failure rounds are answered in the serve loop before
        // the SSE body is built; they never reach this shaping.
        Round::Hold(_) | Round::Fail500 => serde_json::Value::Null,
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
    let held = Arc::new(AtomicBool::new(false));
    let held_flag = Arc::clone(&held);
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
            match round(&request) {
                Round::Hold(marker) => {
                    let chunk = serde_json::json!({"choices": [{"delta": {"content": marker}}]});
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {chunk}\n\n"
                    );
                    let _ = stream.flush();
                    held_flag.store(true, Ordering::SeqCst);
                    let _ = stream.read_to_end(&mut Vec::new());
                }
                Round::Fail500 => {
                    let _ = write!(
                        stream,
                        "HTTP/1.1 500 Internal Server Error\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    );
                    let _ = stream.flush();
                }
                round => {
                    let body = sse(&round, index);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.flush();
                }
            }
        }
    });
    Scripted {
        address,
        bodies,
        held,
    }
}

/// Blocks until a hold round has flushed its first chunk: the client is
/// mid-request and the test may cancel. Panics at the deadline.
pub fn wait_until_holding(scripted: &Scripted, deadline: Duration) {
    let start = Instant::now();
    while !scripted.held.load(Ordering::SeqCst) {
        if start.elapsed() >= deadline {
            panic!("the hold round never started streaming");
        }
        thread::sleep(Duration::from_millis(5));
    }
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

/// The stderr of one blocking run: the diagnostics a scripted run said.
pub fn stderr(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The shared headless invocation: `saya --workspace <project> --turn-file
/// <turn> --approval-mode <mode> --allow-data-sharing --format ndjson` plus
/// `extra` arguments (`--allow` seeds, `--resume`), with NO connections file
/// and NO profile, isolated config/state/session dirs, and the scripted
/// provider behind `SAYA_PROVIDER_BASE_URL`. stdin is null throughout: the
/// run never prompts.
fn run_with_extra(
    env: &Env,
    scripted: &Scripted,
    mode: &str,
    turn: &str,
    extra: &[&str],
) -> std::process::Output {
    let turn_file = env.root.join("turn.txt");
    std::fs::write(&turn_file, turn).unwrap();
    let mut args = vec![
        "--workspace",
        env.project.to_str().unwrap(),
        "--turn-file",
        turn_file.to_str().unwrap(),
        "--approval-mode",
        mode,
        "--allow-data-sharing",
        "--format",
        "ndjson",
    ];
    args.extend_from_slice(extra);
    Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(args)
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

/// `run_with_extra` with no launch grant: the unseeded run.
pub fn run(env: &Env, scripted: &Scripted, mode: &str, turn: &str) -> std::process::Output {
    run_with_extra(env, scripted, mode, turn, &[])
}

/// `run` plus one `--allow <token>` per entry: the launch statement's grant
/// seeds, landed in the grant store on a fresh start (`seed_launch_allow`).
pub fn run_allowing(
    env: &Env,
    scripted: &Scripted,
    mode: &str,
    turn: &str,
    allow: &[&str],
) -> std::process::Output {
    let mut extra = Vec::new();
    for token in allow {
        extra.push("--allow");
        extra.push(token);
    }
    run_with_extra(env, scripted, mode, turn, &extra)
}

/// A resumed run: `--resume <id>` re-opens the saved record from the same
/// session dir. `allow` seeds are empty for the contract under test —
/// grants die with the process — and carried only by the mutation that
/// proves the test bites.
pub fn run_resuming(
    env: &Env,
    scripted: &Scripted,
    mode: &str,
    turn: &str,
    session_id: &str,
    allow: &[&str],
) -> std::process::Output {
    let mut extra = vec!["--resume", session_id];
    for token in allow {
        extra.push("--allow");
        extra.push(token);
    }
    run_with_extra(env, scripted, mode, turn, &extra)
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

/// The last tool-result message of a captured request: the model-facing
/// result of the call the previous round made.
pub fn last_tool_result(raw: &str) -> String {
    tool_results(&request_json(raw))
        .last()
        .copied()
        .expect("the request carries a tool result")
        .to_owned()
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

/// A background turn-file run whose pipes are drained on reader threads and
/// whose child is killed on drop: the shape a test needs to signal SIGINT
/// mid-request and still collect both streams (pattern of InteractiveChildGuard
/// in tests/mvp/session.rs).
pub struct Background {
    child: Option<std::process::Child>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Drop for Background {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().unwrap_or(None).is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

impl Background {
    pub fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Polls the drained stdout buffer until `needle` appears (the child is
    /// provably mid-stream before the test cancels it).
    pub fn wait_for_stdout(&self, needle: &[u8], what: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self
                .stdout
                .lock()
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle)
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("stdout never carried {what}");
    }

    /// Waits for the child to exit; `None` at the deadline.
    pub fn wait_for_exit(&mut self, deadline: Duration) -> Option<std::process::ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                return Some(status);
            }
            if start.elapsed() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout.lock().unwrap()).into_owned()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned()
    }
}

/// `run` with piped, thread-drained stdout/stderr: the same invocation, but
/// the test stays in control of the child's lifetime and signals.
pub fn run_background(env: &Env, scripted: &Scripted, mode: &str, turn: &str) -> Background {
    let turn_file = env.root.join("turn.txt");
    std::fs::write(&turn_file, turn).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stdout_pipe = child.stdout.take().unwrap();
    let stdout_sink = Arc::clone(&stdout);
    thread::spawn(move || {
        let mut reader = stdout_pipe;
        let mut buffer = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut buffer) {
            if size == 0 {
                break;
            }
            stdout_sink
                .lock()
                .unwrap()
                .extend_from_slice(&buffer[..size]);
        }
    });
    let stderr_pipe = child.stderr.take().unwrap();
    let stderr_sink = Arc::clone(&stderr);
    thread::spawn(move || {
        let mut reader = stderr_pipe;
        let mut buffer = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut buffer) {
            if size == 0 {
                break;
            }
            stderr_sink
                .lock()
                .unwrap()
                .extend_from_slice(&buffer[..size]);
        }
    });
    Background {
        child: Some(child),
        stdout,
        stderr,
    }
}

/// Sends SIGINT to the child — the clean cancel channel.
#[cfg(unix)]
pub fn sigint(child_id: u32) {
    assert_eq!(
        unsafe { libc::kill(child_id as libc::pid_t, libc::SIGINT) },
        0,
        "SIGINT was delivered"
    );
}
