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
#[test]
fn a_clarification_reply_after_a_long_idle_wait_is_not_budget_stopped() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::{Child, Command, Stdio},
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
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

    fn reply(stream: &mut std::net::TcpStream, body: &str) {
        let done = format!("data: [{}]\n\n", "DONE");
        let full = format!("{body}{done}");
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{full}"
        )
        .unwrap();
        stream.flush().unwrap();
    }

    let clarification_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,",
        "\"id\":\"call-1\",\"function\":{\"name\":\"request_clarification\",",
        "\"arguments\":\"{\\\"question\\\":\\\"Which one?\\\",\\\"options\\\":[\\\"a\\\",\\\"b\\\"]}\"}}]},",
        "\"finish_reason\":\"tool_calls\"}]}\n\n"
    );
    let answer_body = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"the-final-answer\"},",
        "\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,",
        "\"completion_tokens\":1}}\n\n"
    );
    let harmless_body = "data: {\"content\":\"{\\\"proposals\\\": []}\"}\n\n";

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let server_count = requests.clone();
    let clarification_owned = clarification_body.to_owned();
    let answer_owned = answer_body.to_owned();
    let harmless_owned = harmless_body.to_owned();
    let server = thread::spawn(move || {
        loop {
            let Some(mut stream) = accept_provider(&listener) else {
                break;
            };
            drain_provider_request(&mut stream);
            let index = server_count.fetch_add(1, Ordering::SeqCst);
            if index == 0 {
                reply(&mut stream, &clarification_owned);
            } else if index == 1 {
                reply(&mut stream, &answer_owned);
            } else {
                reply(&mut stream, &harmless_owned);
            }
            drop(stream);
            if index >= 8 {
                break;
            }
        }
    });

    let pid = std::process::id();
    let config_root = std::env::temp_dir().join(format!("saya-cli-question-budget-{pid}"));
    let session_root = std::env::temp_dir().join(format!("saya-cli-question-session-{pid}"));
    std::fs::create_dir_all(config_root.join("saya")).unwrap();
    std::fs::write(
        config_root.join("saya").join("config.toml"),
        "[ai]\ninvestigation_max_elapsed_seconds = 2\n",
    )
    .unwrap();

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
    stdin.write_all(b"which one do you mean\n").unwrap();
    stdin.flush().unwrap();

    wait_for_stream(&stdout_buf, "⏸ question".as_bytes(), "pending clarification");
    // The question is parked awaiting the human reply, so the 3s idle wait
    // here must not count against the 2s question elapsed budget.
    thread::sleep(Duration::from_secs(3));
    stdin.write_all(b"option a\n").unwrap();
    stdin.flush().unwrap();

    wait_for_stream(&stdout_buf, b"the-final-answer", "final answer");
    thread::sleep(Duration::from_secs(2));
    let stderr_text = String::from_utf8_lossy(&stderr_buf.lock().unwrap()).into_owned();
    assert!(
        !stderr_text.contains("request cancelled"),
        "budget stopped the reply turn: {stderr_text}"
    );

    stdin.write_all(b"/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    wait_with_deadline(child, Duration::from_secs(5));
    let status = guard.0.take().unwrap().wait().unwrap();
    assert!(status.success(), "child should exit cleanly");
    let _ = stderr_pump.join();
    let _ = stdout_pump.join();
    server.join().unwrap();
    assert!(
        requests.load(Ordering::SeqCst) >= 2,
        "mock should see question + reply turns"
    );
    let _ = std::fs::remove_dir_all(config_root);
    let _ = std::fs::remove_dir_all(session_root);
}






