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
