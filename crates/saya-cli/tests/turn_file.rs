//! H-S1 `--turn-file`: one verbatim turn, then exit.
//!
//! The only headless path into a session today is piped stdin read
//! line-by-line (`session_loop.rs`), which folds multi-line markdown.
//! These tests pin the new flag: the file bytes reach the turn unaltered,
//! exactly one turn runs, the process exits, and the exit code reflects the
//! turn's outcome. The last test pins today's piped path unchanged.

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

struct TestEnv {
    root: PathBuf,
    connections: PathBuf,
    sessions: PathBuf,
}

fn test_root(name: &str) -> TestEnv {
    let root = std::env::temp_dir().join(format!(
        "saya-cli-turn-file-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("turn.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE t (id INTEGER); INSERT INTO t VALUES (1);")
        .unwrap();
    let connections = root.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'duckdb'\npath = '{}'\nread_only = true\n",
            database.display()
        ),
    )
    .unwrap();
    let sessions = root.join("sessions");
    std::fs::create_dir_all(&sessions).unwrap();
    TestEnv {
        root,
        connections,
        sessions,
    }
}

/// Mock OpenAI-compatible SSE endpoint. Captures the first request body
/// verbatim and answers every round with `answer`; returns the listener
/// address plus the captured bodies.
fn mock(answer: &str) -> (String, std::sync::Arc<std::sync::Mutex<Vec<Vec<u8>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let bodies = std::sync::Arc::new(std::sync::Mutex::new(Vec::<Vec<u8>>::new()));
    let seen = std::sync::Arc::clone(&bodies);
    let answer = answer.to_owned();
    thread::spawn(move || {
        for _ in 0..16 {
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
            seen.lock().unwrap().push(raw);
            let chunk = serde_json::json!({"choices": [{"delta": {"content": answer}}]});
            let body = format!("data: {chunk}\n\ndata: [DONE]\n\n");
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.flush();
        }
    });
    (address, bodies)
}

fn saya(env: &TestEnv, address: &str, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args(args)
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn base_args(env: &TestEnv) -> Vec<String> {
    vec![
        "--approval-mode".into(),
        "read-only".into(),
        "--allow-data-sharing".into(),
        "--format".into(),
        "ndjson".into(),
        "--connections".into(),
        env.connections.to_str().unwrap().into(),
        "--profile".into(),
        "local".into(),
    ]
}

/// Base arguments without the explicit `--approval-mode`: a resumed session
/// then keeps its persisted mode, which is the seam the unparsable-mode
/// repro rides (an explicit flag would override the record at resume).
fn resume_args(env: &TestEnv, format: &str) -> Vec<String> {
    vec![
        "--allow-data-sharing".into(),
        "--format".into(),
        format.into(),
        "--connections".into(),
        env.connections.to_str().unwrap().into(),
        "--profile".into(),
        "local".into(),
    ]
}
/// The first `wt-…` worktree label in a listing — 67 chars, `wt-` plus 64
/// lowercase hex.
fn first_worktree_label(text: &str) -> Option<String> {
    text.split_whitespace()
        .find(|token| token.len() == 67 && token.starts_with("wt-"))
        .map(str::to_owned)
}
#[path = "turn_file/activation_notice.rs"]
mod activation_notice;
#[path = "turn_file/ask_context.rs"]
mod ask_context;
#[path = "turn_file/failed_turns.rs"]
mod failed_turns;
#[path = "turn_file/launch_grants.rs"]
mod launch_grants;
#[path = "turn_file/preferences.rs"]
mod preferences;
#[path = "turn_file/recovery.rs"]
mod recovery;
#[path = "turn_file/rotation_limits.rs"]
mod rotation_limits;
#[path = "turn_file/session_storage.rs"]
mod session_storage;
#[path = "turn_file/turn_execution.rs"]
mod turn_execution;
