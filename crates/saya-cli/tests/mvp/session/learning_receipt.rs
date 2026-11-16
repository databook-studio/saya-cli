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
