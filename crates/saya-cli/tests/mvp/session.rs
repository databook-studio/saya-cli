use saya_cli::{GlobalOptions, approval_name};

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
    std::fs::remove_dir_all(root).unwrap();
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
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn active_sigint_cancels_request_without_persisting_incomplete_turn() {
    use std::{
        io::{self, Read, Write},
        net::TcpListener,
        process::{Child, Command, Stdio},
        sync::mpsc,
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

    // Mock provider: accept one connection but never answer, so the agent's
    // request stays in flight until it is cancelled.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (accepted_tx, accepted_rx) = mpsc::channel();
    let server = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(value) => break value,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                Err(_) => return,
            }
        };
        accepted_tx.send(()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let _ = stream.read_to_end(&mut request);
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
    let mut guard = ChildGuard(Some(command.spawn().unwrap()));
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
    let stdout_pump = thread::spawn(move || {
        let mut reader = stdout;
        let mut sink = Vec::new();
        let _ = reader.read_to_end(&mut sink);
    });

    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(b"incomplete prompt\n").unwrap();
    stdin.flush().unwrap();

    // The request reached the provider and is now hanging.
    accepted_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("provider request was not observed");

    // SIGINT must be caught (cancelling the request), not kill the process.
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );

    // Give the loop a moment to cancel and return, then exit cleanly.
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        let seen = stderr_buf.lock().unwrap();
        if seen
            .windows(b"Request cancelled.".len())
            .any(|window| window == b"Request cancelled.")
        {
            break;
        }
        drop(seen);
        thread::sleep(Duration::from_millis(20));
    }
    stdin.write_all(b"/exit\n").unwrap();
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
    assert_eq!(value["turns"].as_array().unwrap().len(), 0);
    assert!(!saved.contains("incomplete prompt"));
    assert!(!saved.contains("mock-secret"));
    let _ = std::fs::remove_dir_all(config_root);
    std::fs::remove_dir_all(session_root).unwrap();
}
