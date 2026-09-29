//! M0-5: chart temp-file lifecycle.
//!
//! `render_chart` writes `saya-chart-*.html` into the temp directory and
//! nothing removed them. The session teardown must delete every chart temp
//! file the session wrote: the test drives a chart render through the real
//! binary on the piped-REPL path (the session-loop funnel), ends the session,
//! and asserts no `saya-chart-*.html` remains.
//!
//! The run is isolated (TA-02): the child gets its own temp directory via
//! `TMPDIR` (plus the Windows equivalents `TMP`/`TEMP`), and every scan and
//! clean below is scoped to that private directory — the test never deletes
//! or counts chart files in the shared temp directory another process may be
//! using. To pin that, a foreign `saya-chart-*.html` is planted in the
//! process's real temp directory (what `std::env::temp_dir()` returns here)
//! before the run and must still exist afterwards.

use std::{
    io::{Read, Write},
    net::TcpListener,
    path::{Path, PathBuf},
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

fn chart_temp_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
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

fn clear_chart_temp_files_in(dir: &Path) {
    for path in chart_temp_files_in(dir) {
        let _ = std::fs::remove_file(path);
    }
}

/// A foreign `saya-chart-*.html` planted in the process's real temp directory
/// — the shared directory where another live saya session would legitimately
/// write its own charts. Dropping the guard removes the plant, including when
/// the test panics, so the plant never leaks into the shared directory.
struct ForeignChartGuard {
    path: PathBuf,
}

impl Drop for ForeignChartGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn plant_foreign_chart() -> ForeignChartGuard {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    // "foreign" cannot appear in a production-reserved name
    // (`saya-chart-{16-hex}.html`), so the plant can never collide with a
    // live session's recorded chart.
    let path = std::env::temp_dir().join(format!(
        "saya-chart-foreign-{}-{nonce}.html",
        std::process::id()
    ));
    std::fs::write(&path, "planted by chart_cleanup.rs; must survive the run").unwrap();
    ForeignChartGuard { path }
}

/// Waits for the run to finish (the `complete` event) and then asserts the
/// hidden chart call failed validation — never a denial, never a completion:
/// an unadvertised name is unknown to the loop, so nothing approves it and
/// nothing executes it.
fn wait_for_chart_validation_failure(
    buffer: &Arc<Mutex<String>>,
    stderr_text: &Arc<Mutex<String>>,
    timeout: Duration,
) {
    let deadline = Instant::now() + timeout;
    loop {
        let seen = buffer.lock().unwrap().clone();
        if seen
            .lines()
            .any(|line| line.contains("\"event\":\"complete\""))
        {
            drop(seen);
            let done = buffer.lock().unwrap().clone();
            assert!(
                done.lines()
                    .any(|line| line.contains("\"name\":\"render_chart\"")
                        && line.contains("failed validation")),
                "a hidden render_chart must fail validation, never deny or complete; stdout so far:\n{}\nstderr so far:\n{}",
                done,
                stderr_text.lock().unwrap()
            );
            assert!(
                !done
                    .lines()
                    .any(|line| line.contains("\"event\":\"tool_denied\"")
                        && line.contains("render_chart")),
                "a hidden tool is unknown, not denied; stdout so far:\n{}\nstderr so far:\n{}",
                done,
                stderr_text.lock().unwrap()
            );
            return;
        }
        drop(seen);
        assert!(
            Instant::now() < deadline,
            "the run never completed; stdout so far:\n{}\nstderr so far:\n{}",
            buffer.lock().unwrap(),
            stderr_text.lock().unwrap()
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn read_only_hides_render_chart_headlessly_and_nothing_is_written() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());

    // Round 1: one render_chart tool call against the test database — the
    // canned hallucination the mock always issues, even when the tool is
    // hidden. An unadvertised name fails validation before approval or
    // execution, which is exactly the hidden-not-advertised outcome: no
    // denial, no file, no browser.
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
    // The child's own temp directory, handed to it via TMPDIR: every chart
    // file the session could write lands here, so all scans and cleans below
    // stay inside this test's private directory (TA-02).
    let child_temp = root.join("tmp");
    std::fs::create_dir_all(&child_temp).unwrap();
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

    // A foreign chart planted in the shared temp directory — where another
    // live saya session would write its own charts — must survive the whole
    // run: this test cleans only its private directory.
    let foreign = plant_foreign_chart();

    // Stale leftovers from any earlier crashed run must not mask the verdict;
    // the sweep is scoped to the private directory and never touches the
    // shared temp directory (the planted foreign file above proves it).
    clear_chart_temp_files_in(&child_temp);

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
        // The child's temp directory is private to this run: `TMPDIR` on
        // Unix, `TMP`/`TEMP` as the Windows equivalents.
        .env("TMPDIR", &child_temp)
        .env("TMP", &child_temp)
        .env("TEMP", &child_temp)
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
    // external side effect — and read-only approval denies exactly that, so
    // the chart stays hidden there, not advertised-and-denied: the request
    // the mock sends names no advertised tool, so it fails validation before
    // approval or execution — no denial, no file, no browser. This headless
    // run therefore cannot produce a chart at all, which is the intended
    // outcome of the M0-1 gate rather than a regression: a side-effecting
    // tool must not auto-run without a person saying yes. What this
    // end-to-end run pins is the validation failure (not a denial) plus no
    // chart temp file. Cleanup itself is covered directly by the unit tests
    // in `chart/cleanup.rs`.
    wait_for_chart_validation_failure(&accumulated, &stderr_accum, Duration::from_secs(30));
    let written = chart_temp_files_in(&child_temp);
    assert!(
        written.is_empty(),
        "a hidden render_chart must not have written anything; stdout:\n{}",
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

    let remaining = chart_temp_files_in(&child_temp);
    assert!(
        remaining.is_empty(),
        "chart temp files remain after session teardown: {remaining:?}"
    );
    assert!(
        foreign.path.is_file(),
        "the foreign chart planted in the shared temp directory must survive the run: {:?}",
        foreign.path
    );

    let _ = std::fs::remove_dir_all(&root);
    drop(foreign);
}
