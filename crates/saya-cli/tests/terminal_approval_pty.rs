#![cfg(not(windows))]

//! Real-terminal proof for the non-TUI approval input owner.
//!
//! The mock's first received request is the source-start boundary. It returns
//! one SQL tool call, which reaches `TerminalApproval`; the configured
//! question deadline must then cancel that pending raw-mode read and let the
//! child settle without a second provider request.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(15);

#[test]
fn elapsed_deadline_cancels_pending_terminal_approval_without_a_second_request() {
    run_pending_approval_cancellation("deadline", 1, None);
}

#[test]
fn bare_esc_cancels_pending_terminal_approval_before_a_long_deadline() {
    run_pending_approval_cancellation("esc", 60, Some(b"\x1b"));
}

fn run_pending_approval_cancellation(label: &str, elapsed_seconds: u64, input: Option<&[u8]>) {
    let root = scratch_root(label);
    let _cleanup = RootGuard(root.clone());
    let database = root.join("data.sqlite3");
    fs::write(&database, []).unwrap();
    let config_home = root.join("config-home");
    let saya_config = config_home.join("saya");
    fs::create_dir_all(&saya_config).unwrap();
    let config = saya_config.join("config.toml");
    fs::write(
        &config,
        format!(
            "[ai]\nmodel = 'mock'\nallow_data_sharing = true\n\
             investigation_max_elapsed_seconds = {elapsed_seconds}\n\n\
             [memory]\nmode = 'off'\n"
        ),
    )
    .unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    let mut provider = MockProvider::spawn();
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("native pty allocates");
    let mut command = CommandBuilder::new("sh");
    command.cwd(&root);
    command.args([
        "-c",
        "\"$@\"; status=$?; stty -a; IFS= read -r next; printf 'NEXT=%s\\n' \"$next\"; exit \"$status\"",
        "terminal-approval-pty",
        env!("CARGO_BIN_EXE_saya"),
        "--connections",
        connections.to_str().unwrap(),
        "--profile",
        "local",
        "--approval-mode",
        "ask",
        "--allow-data-sharing",
        "ask",
        "count rows",
    ]);
    command.env("HOME", &root);
    command.env("SAYA_CONFIG_HOME", &config_home);
    command.env("SAYA_STATE_DB", root.join("state.sqlite3"));
    command.env("SAYA_PROVIDER", "openai_compatible");
    command.env("SAYA_MODEL", "mock");
    command.env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", provider.address));
    command.env("SAYA_API_KEY", "test-only");
    let mut child = pair.slave.spawn_command(command).expect("saya starts");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    let (tx, rx) = std::sync::mpsc::channel();
    thread::spawn(move || {
        let mut bytes = [0_u8; 4096];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 || tx.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    });

    let started = Instant::now();
    let mut terminal = String::new();
    while !terminal.contains("[a] allow once") {
        assert!(
            started.elapsed() < DEADLINE,
            "approval never became pending; terminal:\n{terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert_eq!(
        provider.requests(),
        1,
        "the real provider source started once"
    );
    if let Some(input) = input {
        writer
            .write_all(input)
            .expect("bare Esc reaches the terminal owner");
    }

    while !terminal.contains("icanon") {
        assert!(
            started.elapsed() < DEADLINE,
            "deadline did not restore terminal ownership; terminal:\n{terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    writer
        .write_all(b"next question\r")
        .expect("the next tty owner accepts its line");
    while child.try_wait().expect("child is pollable").is_none() {
        assert!(
            started.elapsed() < DEADLINE,
            "terminal wrapper did not settle"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    let status = child.try_wait().unwrap().expect("child settled");
    let expected_exit = if input.is_some() { 5 } else { 0 };
    assert_eq!(
        status.exit_code(),
        expected_exit,
        "cancelled question has its established exit class: {terminal}"
    );
    if input.is_some() {
        assert!(
            terminal.contains("request cancelled"),
            "bare Esc must reach the shared cancellation path: {terminal}"
        );
    }
    assert_eq!(
        provider.requests(),
        1,
        "cancellation while approval waited must not start another provider/tool round"
    );
    assert!(
        !terminal.contains("Tool completed"),
        "an unapproved, cancelled call cannot be promoted as successful evidence: {terminal}"
    );
    while let Ok(bytes) = rx.recv_timeout(Duration::from_secs(1)) {
        terminal.push_str(&String::from_utf8_lossy(&bytes));
        if terminal.contains("icanon") {
            break;
        }
    }
    assert!(
        terminal.contains("icanon") && !terminal.contains("-icanon"),
        "the approval guard must restore canonical terminal mode: {terminal}"
    );
    assert!(
        terminal.contains("NEXT=next question"),
        "the next tty owner, not the cancelled approval, must receive its line: {terminal}"
    );
    provider.join();
}

struct MockProvider {
    address: String,
    requests: Arc<Mutex<usize>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockProvider {
    fn spawn() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(0));
        let recorded = Arc::clone(&requests);
        let thread = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("provider receives request");
            drain_request(&mut stream);
            *recorded.lock().unwrap() += 1;
            let response = tool_response();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .unwrap();
            stream.flush().unwrap();
        });
        Self {
            address,
            requests,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> usize {
        *self.requests.lock().unwrap()
    }

    fn join(&mut self) {
        self.thread.take().unwrap().join().unwrap();
    }
}

fn drain_request(stream: &mut std::net::TcpStream) {
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let content_length = String::from_utf8_lossy(&bytes[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + content_length {
                return;
            }
        }
        let Ok(count) = stream.read(&mut chunk) else {
            return;
        };
        if count == 0 {
            return;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

fn tool_response() -> String {
    let chunk = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": "call-1",
            "function": {
                "name": "bounded_sql_query",
                "arguments": "{\"sql\":\"SELECT 1\"}"
            }
        }]}}]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

fn scratch_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-terminal-approval-pty-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
