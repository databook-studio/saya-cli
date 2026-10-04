use saya_cli::{GlobalOptions, approval_name};

#[cfg(unix)]
struct InteractiveChildGuard(Option<std::process::Child>);

#[cfg(unix)]
impl Drop for InteractiveChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

#[cfg(unix)]
fn drain_provider_request(stream: &mut std::net::TcpStream) -> String {
    use std::io::Read;

    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let length = String::from_utf8_lossy(&bytes[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + length {
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + length]).into_owned();
            }
        }
        let count = match stream.read(&mut chunk) {
            Ok(count) => count,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return String::from_utf8_lossy(&bytes).into_owned();
            }
            Err(error) => panic!("provider request reads: {error}"),
        };
        if count == 0 {
            return String::new();
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
}

#[cfg(unix)]
fn accept_provider(listener: &std::net::TcpListener) -> Option<std::net::TcpStream> {
    use std::{
        io, thread,
        time::{Duration, Instant},
    };

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).ok()?;
                stream.set_read_timeout(Some(Duration::from_secs(5))).ok()?;
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .ok()?;
                return Some(stream);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

#[cfg(unix)]
fn wait_for_stream(buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>, needle: &[u8], label: &str) {
    use std::{
        thread,
        time::{Duration, Instant},
    };

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if buffer
            .lock()
            .unwrap()
            .windows(needle.len())
            .any(|window| window == needle)
        {
            return;
        }
        assert!(Instant::now() < deadline, "did not observe {label}");
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn tool_collection_limit_frame() -> String {
    let calls = (0..=256)
        .map(|index| {
            format!(
                "{{\"index\":{index},\"id\":\"limit-{index}\",\"function\":{{\"name\":\"bounded_sql_query\",\"arguments\":\"{{}}\"}}}}"
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!("data: {{\"choices\":[{{\"delta\":{{\"tool_calls\":[{calls}]}}}}]}}\n\n")
}

#[test]
fn scripted_repl_persists_and_continues_a_redacted_session() {
    use std::io::Write;
    let root = std::env::temp_dir().join(format!("saya-cli-repl-{}", std::process::id()));
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .args(["--format", "ndjson"])
        .env("SAYA_SESSION_DIR", &root)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"/help\n/exit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("/help"));
    assert!(std::fs::read_dir(&root).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|ext| ext == "json")
    }));
    let resumed = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .env("SAYA_SESSION_DIR", &root)
        .arg("--continue")
        .output()
        .unwrap();
    assert!(resumed.status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn non_interactive_mode_does_not_start_a_prompt() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .arg("--non-interactive")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires a subcommand"));
}

#[test]
fn non_interactive_defaults_to_never_approval_but_interactive_defaults_to_ask() {
    let interactive = GlobalOptions::default();
    let non_interactive = GlobalOptions {
        non_interactive: true,
        ..Default::default()
    };
    assert_eq!(approval_name(&interactive).unwrap(), "ask");
    assert_eq!(approval_name(&non_interactive).unwrap(), "never");
    let explicit = GlobalOptions {
        non_interactive: true,
        approval_mode: Some("read-only".into()),
        ..Default::default()
    };
    assert_eq!(approval_name(&explicit).unwrap(), "read-only");
}

#[test]
fn idle_eof_exits_the_interactive_process_cleanly() {
    let root = std::env::temp_dir().join(format!("saya-cli-eof-{}", std::process::id()));
    let child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .env("SAYA_SESSION_DIR", &root)
        .spawn()
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let _ = std::fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn active_sigint_cancels_request_without_persisting_incomplete_turn() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::{Child, Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    fn wait_with_deadline(child: &mut Child, timeout: Duration) {
        let deadline = Instant::now() + timeout;
        loop {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(Instant::now() < deadline, "interactive child did not exit");
            thread::sleep(Duration::from_millis(20));
        }
    }

    // Mock provider: the first completed answer has the only paid receipt.
    // The second response remains an unfinished marker-only SSE stream until
    // the real session path receives SIGINT.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (first_done_tx, first_done_rx) = mpsc::channel();
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let Some(mut first) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut first);
        let first_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"first answer\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{first_body}",
            first_body.len()
        )
        .unwrap();
        first.flush().unwrap();
        first_done_tx.send(()).unwrap();

        let Some(mut second) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut second);
        write!(
            second,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n"
        )
        .unwrap();
        second
            .write_all(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"active-cancel-marker\"}}]}\n\n",
            )
            .unwrap();
        second.flush().unwrap();
        accepted_tx.send(()).unwrap();
        let _ = second.read_to_end(&mut Vec::new());
    });

    let config_root =
        std::env::temp_dir().join(format!("saya-cli-cancel-config-{}", std::process::id()));
    let session_root =
        std::env::temp_dir().join(format!("saya-cli-cancel-session-{}", std::process::id()));

    // Piped stdin/stdout makes this the headless (non-TTY) execution path, which
    // is where scripts and CI run and which keeps the SIGINT cancellation.
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("SAYA_CONFIG_HOME", &config_root)
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    let mut guard = InteractiveChildGuard(Some(command.spawn().unwrap()));
    let child = guard.0.as_mut().unwrap();

    // Drain stdout/stderr so the child never blocks on a full pipe.
    let stderr = child.stderr.take().unwrap();
    let stderr_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let stderr_sink = stderr_buf.clone();
    let stderr_pump = thread::spawn(move || {
        let mut reader = stderr;
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
    let stdout = child.stdout.take().unwrap();
    let stdout_buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let stdout_sink = stdout_buf.clone();
    let stdout_pump = thread::spawn(move || {
        let mut reader = stdout;
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

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"completed prompt\n").unwrap();
    stdin.flush().unwrap();

    // The source reported the first settled response; only then ask the
    // session for its second, cancellable request.
    first_done_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("first provider response was not written");
    stdin.write_all(b"incomplete prompt\n").unwrap();
    stdin.flush().unwrap();

    // The source has an unfinished response open; the marker proves the child
    // consumed that live stream before SIGINT.
    accepted_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("provider request was not observed");
    wait_for_stream(
        &stdout_buf,
        b"active-cancel-marker",
        "active cancellation source marker",
    );

    // SIGINT must be caught (cancelling the request), not kill the process.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );

    wait_for_stream(&stderr_buf, b"Request cancelled.", "cancelled action");
    stdin.write_all(b"/usage\n/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    wait_with_deadline(child, Duration::from_secs(5));
    let status = guard.0.take().unwrap().wait().unwrap();
    assert!(
        status.success(),
        "SIGINT should be handled, not terminate the process"
    );
    let _ = stderr_pump.join();
    let _ = stdout_pump.join();
    server.join().unwrap();

    let saved = std::fs::read_dir(&session_root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap();
    let value: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(value["turns"].as_array().unwrap().len(), 1);
    assert!(!saved.contains("incomplete prompt"));
    assert!(!saved.contains("mock-secret"));
    let usage = String::from_utf8_lossy(&stdout_buf.lock().unwrap()).into_owned();
    assert!(usage.contains("Input tokens: 10"), "usage: {usage}");
    assert!(usage.contains("Output tokens: 2"), "usage: {usage}");
    assert!(usage.contains("1 turn"), "usage: {usage}");
    let _ = std::fs::remove_dir_all(config_root);
    let _ = std::fs::remove_dir_all(session_root);
}

#[cfg(unix)]
#[test]
fn terminal_provider_error_keeps_same_question_settled_usage_once() {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        process::{Command, Stdio},
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    let root = std::env::temp_dir().join(format!(
        "saya-cli-terminal-usage-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    fs::File::create(&database).unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (second_request_tx, second_request_rx) = mpsc::channel();
    let (marker_seen_tx, marker_seen_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let Some(mut first) = accept_provider(&listener) else {
            return;
        };
        first
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        drain_provider_request(&mut first);
        let first_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"sql-1\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"sql\\\":\\\"SELECT 1 AS value\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{first_body}",
            first_body.len()
        )
        .unwrap();
        first.flush().unwrap();

        let Some(mut second) = accept_provider(&listener) else {
            return;
        };
        second
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        drain_provider_request(&mut second);
        second_request_tx.send(()).unwrap();
        write!(
            second,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        // `Usage` is queued before the visible delta. Seeing the delta in the
        // child proves the real stream delivered that earlier receipt; only
        // then does this source issue its terminal collection-limit error.
        second
            .write_all(
                b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"terminal-marker\"}}]}\n\n",
            )
            .unwrap();
        second.flush().unwrap();
        if marker_seen_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            return;
        }
        second
            .write_all(tool_collection_limit_frame().as_bytes())
            .unwrap();
        second.flush().unwrap();
        drop(second);

        let Some(mut third) = accept_provider(&listener) else {
            return;
        };
        third
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        drain_provider_request(&mut third);
        let third_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"normal-marker\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            third,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{third_body}",
            third_body.len()
        )
        .unwrap();
        third.flush().unwrap();
    });

    let session_root = root.join("sessions");
    let state_db = root.join("state.sqlite3");
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--format",
            "text",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "--profile",
            "local",
            "--connections",
            connections.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_STATE_DB", &state_db)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    let mut guard = InteractiveChildGuard(Some(command.spawn().unwrap()));
    let (mut stdin, stdout_reader, stderr_reader) = {
        let child = guard.0.as_mut().unwrap();
        (
            child.stdin.take().unwrap(),
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
        )
    };
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_sink = stdout.clone();
    let stdout_pump = thread::spawn(move || {
        let mut reader = stdout_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stdout_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_sink = stderr.clone();
    let stderr_pump = thread::spawn(move || {
        let mut reader = stderr_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stderr_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });
    stdin.write_all(b"run a safe query\n").unwrap();
    stdin.flush().unwrap();

    second_request_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the completed SQLite tool should prompt the next provider response");
    wait_for_stream(&stdout, b"terminal-marker", "second response marker");
    marker_seen_tx.send(()).unwrap();
    wait_for_stream(&stderr, b"tool call", "terminal provider error");
    stdin.write_all(b"complete normally\n").unwrap();
    stdin.flush().unwrap();
    wait_for_stream(&stdout, b"normal-marker", "normal completion marker");
    server.join().unwrap();

    stdin.write_all(b"/usage\n/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(5);
    while guard.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "interactive child did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    let status = guard.0.take().unwrap().wait().unwrap();
    assert!(
        status.success(),
        "terminal source error stays in the session"
    );
    stdout_pump.join().unwrap();
    stderr_pump.join().unwrap();

    let output = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    let errors = String::from_utf8_lossy(&stderr.lock().unwrap()).into_owned();
    assert!(output.contains("bounded_sql_query"), "output: {output}");
    assert!(errors.contains("tool call"), "stderr: {errors}");
    assert!(output.contains("Input tokens: 21"), "usage: {output}");
    assert!(output.contains("Output tokens: 6"), "usage: {output}");
    assert!(output.contains("2 turns"), "usage: {output}");
    assert!(!output.contains("3 turns"), "usage double folded: {output}");
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn terminal_usage_distinguishes_absent_from_reported_zero() {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        process::{Command, Stdio},
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    let root = std::env::temp_dir().join(format!(
        "saya-cli-terminal-presence-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (release_tx, release_rx) = mpsc::channel();
    let (zero_request_tx, zero_request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let Some(mut absent) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut absent);
        absent
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"absent-marker\"}}]}\n\n",
            )
            .unwrap();
        absent.flush().unwrap();
        if release_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            return;
        }
        absent
            .write_all(tool_collection_limit_frame().as_bytes())
            .unwrap();
        absent.flush().unwrap();
        drop(absent);

        let Some(mut zero) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut zero);
        zero_request_tx.send(()).unwrap();
        zero.write_all(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\ndata: {\"choices\":[{\"delta\":{\"content\":\"zero-marker\"}}],\"usage\":{\"prompt_tokens\":0,\"completion_tokens\":0}}\n\n",
        )
        .unwrap();
        zero.flush().unwrap();
        if release_rx.recv_timeout(Duration::from_secs(5)).is_err() {
            return;
        }
        zero.write_all(tool_collection_limit_frame().as_bytes())
            .unwrap();
        zero.flush().unwrap();
    });

    let session_root = root.join("sessions");
    let state_db = root.join("state.sqlite3");
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_STATE_DB", &state_db)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    let mut guard = InteractiveChildGuard(Some(command.spawn().unwrap()));
    let (mut stdin, stdout_reader, stderr_reader) = {
        let child = guard.0.as_mut().unwrap();
        (
            child.stdin.take().unwrap(),
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
        )
    };
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_sink = stdout.clone();
    let stdout_pump = thread::spawn(move || {
        let mut reader = stdout_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stdout_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });
    let stderr_pump = thread::spawn(move || {
        let mut reader = stderr_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
        }
    });

    stdin.write_all(b"absent usage\n").unwrap();
    stdin.flush().unwrap();
    wait_for_stream(&stdout, b"absent-marker", "absent source marker");
    release_tx.send(()).unwrap();
    stdin.write_all(b"/usage\n").unwrap();
    stdin.flush().unwrap();
    wait_for_stream(
        &stdout,
        b"No token usage reported yet this session.",
        "absence usage result",
    );

    stdin.write_all(b"reported zero usage\n").unwrap();
    stdin.flush().unwrap();
    zero_request_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("reported-zero prompt did not reach the provider");
    wait_for_stream(&stdout, b"zero-marker", "zero source marker");
    release_tx.send(()).unwrap();
    server.join().unwrap();
    stdin.write_all(b"/usage\n/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(5);
    while guard.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "interactive child did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(guard.0.take().unwrap().wait().unwrap().success());
    stdout_pump.join().unwrap();
    stderr_pump.join().unwrap();

    let output = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    assert!(
        output.contains("Session token usage (1 turn)"),
        "usage: {output}"
    );
    assert!(output.contains("Input tokens: 0"), "usage: {output}");
    assert!(output.contains("Output tokens: 0"), "usage: {output}");
    assert!(
        !output.contains("Session token usage (2 turns)"),
        "usage: {output}"
    );
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[test]
fn same_question_sigint_keeps_consumed_failed_attempt_usage() {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        process::{Command, Stdio},
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    let root = std::env::temp_dir().join(format!(
        "saya-cli-cancelled-question-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    fs::File::create(&database).unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (second_request_tx, second_request_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let Some(mut first) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut first);
        let first_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"sql-1\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"sql\\\":\\\"SELECT 1 AS value\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{first_body}",
            first_body.len()
        )
        .unwrap();
        first.flush().unwrap();

        let Some(mut second) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut second);
        second_request_tx.send(()).unwrap();
        second
            .write_all(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let body = b"data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3}}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"cancel-marker\"}}]}\n\n";
        write!(second, "{:X}\r\n", body.len()).unwrap();
        second.write_all(body).unwrap();
        second.write_all(b"\r\n").unwrap();
        second.flush().unwrap();
        second
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = second.read_to_end(&mut Vec::new());
    });

    let session_root = root.join("sessions");
    let state_db = root.join("state.sqlite3");
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--format",
            "text",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "--profile",
            "local",
            "--connections",
            connections.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_STATE_DB", &state_db)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    let mut guard = InteractiveChildGuard(Some(command.spawn().unwrap()));
    let child_id = guard.0.as_ref().unwrap().id();
    let (mut stdin, stdout_reader, stderr_reader) = {
        let child = guard.0.as_mut().unwrap();
        (
            child.stdin.take().unwrap(),
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
        )
    };
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_sink = stdout.clone();
    let stdout_pump = thread::spawn(move || {
        let mut reader = stdout_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stdout_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_sink = stderr.clone();
    let stderr_pump = thread::spawn(move || {
        let mut reader = stderr_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stderr_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });
    stdin.write_all(b"cancel after a safe query\n").unwrap();
    stdin.flush().unwrap();

    second_request_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("completed SQLite tool did not reach its continuation request");
    wait_for_stream(&stdout, b"cancel-marker", "consumed cancellation marker");
    assert_eq!(
        unsafe { libc::kill(child_id as libc::pid_t, libc::SIGINT) },
        0
    );
    wait_for_stream(
        &stderr,
        b"Request cancelled.",
        "settled session cancellation",
    );
    server.join().unwrap();

    stdin.write_all(b"/usage\n/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(5);
    while guard.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "interactive child did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(guard.0.take().unwrap().wait().unwrap().success());
    stdout_pump.join().unwrap();
    stderr_pump.join().unwrap();

    let output = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    assert!(output.contains("bounded_sql_query"), "output: {output}");
    assert!(output.contains("Input tokens: 17"), "usage: {output}");
    assert!(output.contains("Output tokens: 5"), "usage: {output}");
    assert!(
        output.contains("Session token usage (1 turn)"),
        "usage: {output}"
    );
    let saved = fs::read_dir(&session_root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| fs::read_to_string(entry.path()).unwrap())
        .unwrap();
    assert!(!saved.contains("cancel after a safe query"));
    let _ = fs::remove_dir_all(root);
}

#[cfg(unix)]
#[tokio::test]
async fn sigint_keeps_settled_learning_receipt_from_real_extraction() {
    use std::{
        fs,
        io::{Read, Write},
        net::TcpListener,
        process::{Command, Stdio},
        sync::{Arc, Mutex, mpsc},
        thread,
        time::{Duration, Instant},
    };

    fn send_sse(stream: &mut std::net::TcpStream, body: &str) -> std::io::Result<()> {
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )?;
        stream.flush()
    }

    let root = std::env::temp_dir().join(format!(
        "saya-cli-cancelled-learning-{}-{}",
        std::process::id(),
        Instant::now().elapsed().as_nanos()
    ));
    fs::create_dir_all(&root).unwrap();
    let database = root.join("data.sqlite3");
    let seed = sqlx::SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::query("CREATE TABLE orders (id INTEGER)")
        .execute(&seed)
        .await
        .unwrap();
    sqlx::query("INSERT INTO orders (id) VALUES (1)")
        .execute(&seed)
        .await
        .unwrap();
    seed.close().await;
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let config_home = root.join("config-home");
    fs::create_dir_all(config_home.join("saya")).unwrap();
    fs::write(
        config_home.join("saya/config.toml"),
        "[memory]\nmode = 'assisted'\n",
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (extraction_started_tx, extraction_started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let Some(mut first) = accept_provider(&listener) else {
            return 0;
        };
        drain_provider_request(&mut first);
        let tool_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"sql-1\",\"function\":{\"name\":\"bounded_sql_query\",\"arguments\":\"{\\\"sql\\\":\\\"SELECT id FROM orders\\\"}\"}}]}}]}\n\n",
            "data: [DONE]\n\n"
        );
        if send_sse(&mut first, tool_body).is_err() {
            return 1;
        }

        let Some(mut second) = accept_provider(&listener) else {
            return 1;
        };
        drain_provider_request(&mut second);
        let answer_body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"The SQLite value is 1, so remember this answer is database-backed.\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        if send_sse(&mut second, answer_body).is_err() {
            return 2;
        }

        let Some(mut extraction) = accept_provider(&listener) else {
            return 2;
        };
        let request = drain_provider_request(&mut extraction);
        assert!(
            request.contains(r#""response_format":{"type":"json_object"}"#)
                && request.contains("precision schema knowledge extractor"),
            "third request is the actual extraction source: {request}"
        );
        if extraction_started_tx.send(()).is_err()
            || release_rx.recv_timeout(Duration::from_secs(5)).is_err()
        {
            return 3;
        }
        let extraction_body = concat!(
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":1}}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"{\\\"proposals\\\": []}\"}}]}\n\n",
            "data: [DONE]\n\n"
        );
        if send_sse(&mut extraction, extraction_body).is_err() {
            return 3;
        }
        3
    });

    let session_root = root.join("sessions");
    let state_db = root.join("state.sqlite3");
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .args([
            "--format",
            "text",
            "--approval-mode",
            "read-only",
            "--allow-data-sharing",
            "--profile",
            "local",
            "--connections",
            connections.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("SAYA_CONFIG_HOME", &config_home)
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_STATE_DB", &state_db)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    let mut guard = InteractiveChildGuard(Some(command.spawn().unwrap()));
    let child_id = guard.0.as_ref().unwrap().id();
    let (mut stdin, stdout_reader, stderr_reader) = {
        let child = guard.0.as_mut().unwrap();
        (
            child.stdin.take().unwrap(),
            child.stdout.take().unwrap(),
            child.stderr.take().unwrap(),
        )
    };
    let stdout = Arc::new(Mutex::new(Vec::new()));
    let stdout_sink = Arc::clone(&stdout);
    let stdout_pump = thread::spawn(move || {
        let mut reader = stdout_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stdout_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });
    let stderr = Arc::new(Mutex::new(Vec::new()));
    let stderr_sink = Arc::clone(&stderr);
    let stderr_pump = thread::spawn(move || {
        let mut reader = stderr_reader;
        let mut chunk = [0_u8; 4096];
        while let Ok(size) = reader.read(&mut chunk) {
            if size == 0 {
                break;
            }
            stderr_sink
                .lock()
                .unwrap()
                .extend_from_slice(&chunk[..size]);
        }
    });

    stdin
        .write_all(b"remember the SQLite answer after querying it\n")
        .unwrap();
    stdin.flush().unwrap();
    if let Err(error) = extraction_started_rx.recv_timeout(Duration::from_secs(5)) {
        panic!(
            "actual JSON-mode extraction request was not observed: {error}; stdout: {}; stderr: {}",
            String::from_utf8_lossy(&stdout.lock().unwrap()),
            String::from_utf8_lossy(&stderr.lock().unwrap()),
        );
    }
    assert_eq!(
        unsafe { libc::kill(child_id as libc::pid_t, libc::SIGINT) },
        0,
        "SIGINT reaches the live headless session"
    );
    thread::sleep(Duration::from_millis(50));
    release_tx.send(()).unwrap();
    wait_for_stream(
        &stderr,
        b"Request cancelled.",
        "cancelled action after extraction settles",
    );
    assert_eq!(
        server.join().unwrap(),
        3,
        "exactly two answer rounds and extraction ran"
    );

    stdin.write_all(b"/usage\n/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(5);
    while guard.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "interactive child did not exit");
        thread::sleep(Duration::from_millis(10));
    }
    assert!(guard.0.take().unwrap().wait().unwrap().success());
    stdout_pump.join().unwrap();
    stderr_pump.join().unwrap();

    let output = String::from_utf8_lossy(&stdout.lock().unwrap()).into_owned();
    assert!(output.contains("bounded_sql_query"), "output: {output}");
    assert!(
        output.contains("Session token usage (1 turn)"),
        "usage: {output}"
    );
    assert!(output.contains("Input tokens: 7"), "usage: {output}");
    assert!(output.contains("Output tokens: 2"), "usage: {output}");
    assert!(output.contains("Learning call (1 turn)"), "usage: {output}");
    assert!(output.contains("Input tokens: 3"), "usage: {output}");
    assert!(output.contains("Output tokens: 1"), "usage: {output}");
    let saved = fs::read_dir(&session_root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| fs::read_to_string(entry.path()).unwrap())
        .unwrap();
    let session: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(session["turns"].as_array().unwrap().len(), 0);
    assert!(!saved.contains("remember the SQLite answer"));
    let _ = fs::remove_dir_all(root);
}
