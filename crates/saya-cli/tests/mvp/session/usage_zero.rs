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
