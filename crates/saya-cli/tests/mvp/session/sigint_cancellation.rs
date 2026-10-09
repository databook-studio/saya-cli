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
