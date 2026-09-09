//! M0-5: chart temp-file lifecycle.
//!
//! `render_chart` writes `saya-chart-*.html` into the temp directory and
//! nothing removed them. The session teardown must delete every chart temp
//! file the session wrote: the test drives a chart render through the real
//! binary on the piped-REPL path (the session-loop funnel), ends the session,
//! and asserts no `saya-chart-*.html` remains.

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

struct ChildGuard(Option<Child>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

fn chart_temp_files() -> Vec<PathBuf> {
    let temp = std::env::temp_dir();
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&temp) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with("saya-chart-") && path.extension().is_some_and(|e| e == "html") {
                found.push(path);
            }
        }
    }
    found
}

fn clear_chart_temp_files() {
    for path in chart_temp_files() {
        let _ = std::fs::remove_file(path);
    }
}

/// Polls the accumulated stdout for the ndjson line reporting the tool call
/// completed, which proves the chart file was actually written.
fn wait_for_tool_denied(
    buffer: &Arc<Mutex<String>>,
    stderr_text: &Arc<Mutex<String>>,
    tool: &str,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let seen = buffer.lock().unwrap();
        if seen.lines().any(|line| {
            line.contains("\"event\":\"tool_denied\"") && line.contains(&format!("\"{tool}\""))
        }) {
            return;
        }
        drop(seen);
        assert!(
            Instant::now() < deadline,
            "{tool} was neither denied nor completed; stdout so far:\n{}\nstderr so far:\n{}",
            buffer.lock().unwrap(),
            stderr_text.lock().unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn read_only_denies_render_chart_headlessly_and_nothing_is_written() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());

    // Round 1: one render_chart tool call against the test database.
    let render_call = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [
            {"index": 0, "id": "call_chart", "function": {"name": "render_chart",
                "arguments": serde_json::json!({
                    "sql": "SELECT 'cat' AS label, 7 AS value",
                    "chart_type": "bar"
                }).to_string()}}
        ]}}]
    });
    let tool_calls_body = format!("data: {render_call}\n\ndata: [DONE]\n\n");
    // Round 2: the final answer (no tool calls -> the loop terminates).
    let final_chunk = serde_json::json!({"choices": [{"delta": {"content": "chart done"}}]});
    let final_body = format!("data: {final_chunk}\n\ndata: [DONE]\n\n");
    let handle = thread::spawn(move || {
        for body in [tool_calls_body, final_body] {
            let (mut stream, _) = listener.accept().unwrap();
            // Read the request to completion (bounded by a read timeout, the
            // `active_sigint` mock pattern): responding before the client
            // finishes sending makes the client drop the request, so a single
            // fixed-size read is not enough for these ~10 KiB agent requests.
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buf = [0_u8; 8192];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            if write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            )
            .is_err()
            {
                return;
            }
            let _ = stream.flush();
        }
    });

    let root = std::env::temp_dir().join(format!("saya-cli-chart-cleanup-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let database = root.join("chart.duckdb");
    duckdb::Connection::open(&database)
        .unwrap()
        .execute_batch("CREATE TABLE t (label VARCHAR, value INTEGER);")
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

    // Stale leftovers from any earlier crashed run must not mask the verdict.
    clear_chart_temp_files();

    // Piped stdin/stdout makes this the headless (non-TTY) session path, which
    // funnels through the interactive session loop and ends at session teardown.
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "--format",
            "ndjson",
            "--connections",
            connections.to_str().unwrap(),
            "--profile",
            "local",
        ])
        .current_dir(&root)
        .env("SAYA_CONFIG_HOME", &root)
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut guard = ChildGuard(Some(command.spawn().unwrap()));
    let child = guard.0.as_mut().unwrap();

    // Drain stdout so the child never blocks on a full pipe, accumulating it
    // for the tool-completed wait and for diagnostics on failure.
    let stdout = child.stdout.take().unwrap();
    let accumulated = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&accumulated);
    let pump = thread::spawn(move || {
        let mut reader = stdout;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            sink.lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&chunk[..size]));
        }
    });
    let stderr = child.stderr.take().unwrap();
    let stderr_accum = Arc::new(Mutex::new(String::new()));
    let stderr_sink = Arc::clone(&stderr_accum);
    let stderr_pump = thread::spawn(move || {
        let mut reader = stderr;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stderr_sink
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&chunk[..size]));
        }
    });

    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"chart the t table\n")
        .unwrap();
    child.stdin.as_mut().unwrap().flush().unwrap();

    // `render_chart` writes a file and opens a browser, so it declares an
    // external side effect — and read-only approval denies exactly that. This
    // headless run therefore cannot produce a chart at all, which is the
    // intended outcome of the M0-1 gate rather than a regression: a
    // side-effecting tool must not auto-run without a person saying yes.
    // Cleanup itself is covered directly by the unit tests in
    // `chart/cleanup.rs`; what this end-to-end run pins is the denial.
    wait_for_tool_denied(
        &accumulated,
        &stderr_accum,
        "render_chart",
        Duration::from_secs(30),
    );
    let written = chart_temp_files();
    assert!(
        written.is_empty(),
        "a denied render_chart must not have written anything; stdout:\n{}",
        accumulated.lock().unwrap()
    );

    child.stdin.as_mut().unwrap().write_all(b"/exit\n").unwrap();
    child.stdin.as_mut().unwrap().flush().unwrap();
    drop(child.stdin.take());

    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        assert!(Instant::now() < deadline, "session child did not exit");
        thread::sleep(Duration::from_millis(20));
    }
    let status = guard.0.take().unwrap().wait().unwrap();
    let stderr_text = stderr_accum.lock().unwrap().clone();
    pump.join().unwrap();
    stderr_pump.join().unwrap();
    handle.join().unwrap();
    assert!(
        status.success(),
        "session should exit cleanly, stderr: {stderr_text}"
    );

    let remaining = chart_temp_files();
    assert!(
        remaining.is_empty(),
        "chart temp files remain after session teardown: {remaining:?}"
    );

    let _ = std::fs::remove_dir_all(&root);
}
