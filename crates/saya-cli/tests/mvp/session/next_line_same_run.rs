#[cfg(unix)]
#[test]
fn ctrl_c_mid_turn_leaves_the_next_line_answered_on_the_same_run() {
    use std::{
        io::{Read, Write},
        net::TcpListener,
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::{Duration, Instant},
    };

    // The source: the first line is answered and settled, the second is held
    // open after one visible chunk, and the third is answered once the held
    // socket is gone. A second run would open a second job; one job directory
    // at the end is the proof that the next line ran on the same epoch and the
    // same lease.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (answered_tx, answered_rx) = mpsc::channel();
    let (held_tx, held_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let Some(mut first) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut first);
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"first-line-answer\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        first.flush().unwrap();
        answered_tx.send(()).unwrap();

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
            .write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"held-marker\"}}]}\n\n")
            .unwrap();
        second.flush().unwrap();
        held_tx.send(()).unwrap();
        // The cancelled turn drops its socket: this read returns, and the
        // next line's request can be served by the same source.
        let _ = second.read_to_end(&mut Vec::new());

        let Some(mut third) = accept_provider(&listener) else {
            return;
        };
        drain_provider_request(&mut third);
        let body = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"second-line-answer\"}}]}\n\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":4,\"completion_tokens\":1}}\n\n",
            "data: [DONE]\n\n"
        );
        write!(
            third,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        third.flush().unwrap();
    });

    let config_root =
        std::env::temp_dir().join(format!("saya-cli-same-run-config-{}", std::process::id()));
    let session_root =
        std::env::temp_dir().join(format!("saya-cli-same-run-session-{}", std::process::id()));
    let home_root = config_root.join("home");
    let working_dir = config_root.join("cwd");
    std::fs::create_dir_all(&home_root).unwrap();
    std::fs::create_dir_all(&working_dir).unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        // The run's composition resolves the host's Git through PATH; the
        // environment is cleared for hermeticity, so restore the real one.
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("SAYA_CONFIG_HOME", &config_root)
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_STATE_DB", config_root.join("state.sqlite3"))
        .env("HOME", &home_root)
        .current_dir(&working_dir)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    let mut guard = InteractiveChildGuard(Some(command.spawn().unwrap()));
    let child = guard.0.as_mut().unwrap();
    let child_id = child.id();

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
    stdin.write_all(b"first line\n").unwrap();
    stdin.flush().unwrap();
    answered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the first line was not answered");
    wait_for_stream(&stdout_buf, b"first-line-answer", "the first answer");
    assert!(
        String::from_utf8_lossy(&stdout_buf.lock().unwrap()).contains("first-line-answer"),
        "the first line is answered before the stop"
    );

    stdin.write_all(b"second line\n").unwrap();
    stdin.flush().unwrap();
    held_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the held provider request was not observed");
    wait_for_stream(&stdout_buf, b"held-marker", "the held turn's marker");

    // SIGINT mid-turn: that turn only. The run is not cancelled, so the next
    // line is a turn of the same run.
    assert_eq!(
        unsafe { libc::kill(child_id as libc::pid_t, libc::SIGINT) },
        0
    );
    wait_for_stream(&stderr_buf, b"Request cancelled.", "the turn's own stop");

    stdin.write_all(b"third line\n").unwrap();
    stdin.flush().unwrap();
    wait_for_stream(&stdout_buf, b"second-line-answer", "the next line's answer");
    stdin.write_all(b"/exit\n").unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let deadline = Instant::now() + Duration::from_secs(10);
    while guard.0.as_mut().unwrap().try_wait().unwrap().is_none() {
        assert!(Instant::now() < deadline, "interactive child did not exit");
        thread::sleep(Duration::from_millis(20));
    }
    assert!(guard.0.take().unwrap().wait().unwrap().success());
    let _ = stderr_pump.join();
    let _ = stdout_pump.join();
    server.join().unwrap();

    // One job directory under the child's own coding root: the cancelled turn
    // opened none, and the next line reused the run it was already on.
    let jobs = job_dirs(&config_root);
    assert_eq!(
        jobs.len(),
        1,
        "one job for the whole session: {jobs:?} under {}",
        config_root.display()
    );
    let saved = std::fs::read_dir(&session_root)
        .unwrap()
        .filter_map(Result::ok)
        .find(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
        .map(|entry| std::fs::read_to_string(entry.path()).unwrap())
        .unwrap();
    let record: serde_json::Value = serde_json::from_str(&saved).unwrap();
    assert_eq!(
        record["run"]["job"].as_str(),
        Some(jobs[0].as_str()),
        "the record links the one run the lines ran on: {record}"
    );
    assert_eq!(
        record["turns"].as_array().unwrap().len(),
        2,
        "the two answered lines recorded, the cancelled one did not: {record}"
    );
    assert!(!saved.contains("second line"), "the cancelled line is not a turn");
    assert!(!saved.contains("mock-secret"));

    let _ = std::fs::remove_dir_all(config_root);
    let _ = std::fs::remove_dir_all(session_root);
}

/// The job directories a child left under its own coding state root: a job's
/// directory is the one holding its own request.
#[cfg(unix)]
fn job_dirs(config_root: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(config_root.join("coding")) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().join("request.json").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect()
}
