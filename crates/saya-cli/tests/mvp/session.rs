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
fn active_sigint_returns_to_repl_without_persisting_incomplete_turn() {
    use std::{
        ffi::CStr,
        fs::File,
        io::{self, Read, Write},
        net::TcpListener,
        os::fd::{AsRawFd, FromRawFd},
        os::unix::process::CommandExt,
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

    fn pty_pair() -> (File, File) {
        let master_fd = unsafe { libc::posix_openpt(libc::O_RDWR | libc::O_NOCTTY) };
        assert!(
            master_fd >= 0,
            "posix_openpt: {}",
            io::Error::last_os_error()
        );
        assert_eq!(unsafe { libc::grantpt(master_fd) }, 0);
        assert_eq!(unsafe { libc::unlockpt(master_fd) }, 0);
        // Give the PTY a realistic size so the rich line editor can lay out its prompt.
        let winsize = libc::winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        unsafe {
            libc::ioctl(master_fd, libc::TIOCSWINSZ, &winsize);
        }
        let slave_name = unsafe { CStr::from_ptr(libc::ptsname(master_fd)) };
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(slave_name.to_string_lossy().as_ref())
            .unwrap();
        let master = unsafe { File::from_raw_fd(master_fd) };
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0);
        assert_eq!(
            unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) },
            0
        );
        (master, slave)
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
    let (mut master, slave) = pty_pair();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .stdin(Stdio::from(slave.try_clone().unwrap()))
        .stdout(Stdio::from(slave.try_clone().unwrap()))
        .stderr(Stdio::from(slave))
        .env_clear()
        .env("TERM", "xterm-256color")
        .env("SAYA_CONFIG_HOME", &config_root)
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{address}/v1"))
        .env("SAYA_API_KEY", "mock-secret");
    unsafe {
        command.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(target_vendor = "apple")]
            let controlling_terminal = libc::TIOCSCTTY as libc::c_ulong;
            #[cfg(not(target_vendor = "apple"))]
            let controlling_terminal = libc::TIOCSCTTY;
            if libc::ioctl(libc::STDIN_FILENO, controlling_terminal, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut guard = ChildGuard(Some(command.spawn().unwrap()));
    let child = guard.0.as_mut().unwrap();

    // Drive the pseudo-terminal like a real terminal emulator: a background pump
    // drains all output, answers the rich editor's cursor-position queries (ESC[6n)
    // so it can initialize, and accumulates bytes so the test can wait for markers.
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let pump_output = output.clone();
    let mut pump_master = master.try_clone().unwrap();
    let pump = thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut buffer = [0_u8; 4096];
        while Instant::now() < deadline {
            match pump_master.read(&mut buffer) {
                Ok(0) => break,
                Ok(size) => {
                    let chunk = &buffer[..size];
                    pump_output.lock().unwrap().extend_from_slice(chunk);
                    let queries = chunk
                        .windows(4)
                        .filter(|window| *window == b"\x1b[6n")
                        .count();
                    for _ in 0..queries {
                        let _ = pump_master.write_all(b"\x1b[1;1R");
                        let _ = pump_master.flush();
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(_) => break,
            }
        }
    });

    let wait_for = |needle: &[u8], timeout: Duration| -> Vec<u8> {
        let deadline = Instant::now() + timeout;
        loop {
            {
                let buffer = output.lock().unwrap();
                if buffer.windows(needle.len()).any(|window| window == needle) {
                    return buffer.clone();
                }
                assert!(
                    Instant::now() < deadline,
                    "timed out waiting for {:?}; output was {:?}",
                    String::from_utf8_lossy(needle),
                    String::from_utf8_lossy(&buffer)
                );
            }
            thread::sleep(Duration::from_millis(10));
        }
    };

    // The rich editor renders its prompt (cold debug builds can be slow under load).
    wait_for(b"saya> ", Duration::from_secs(15));
    // Enter is a carriage return in a raw terminal.
    master.write_all(b"incomplete prompt\r").unwrap();
    master.flush().unwrap();
    accepted_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("provider request was not observed");
    assert_eq!(
        unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) },
        0
    );
    let cancelled = wait_for(b"Request cancelled.", Duration::from_secs(5));
    assert!(
        String::from_utf8_lossy(&cancelled).contains("saya> "),
        "{}",
        String::from_utf8_lossy(&cancelled)
    );
    master.write_all(b"/exit\r").unwrap();
    master.flush().unwrap();
    wait_with_deadline(child, Duration::from_secs(5));
    guard.0.take();
    drop(master);
    let _ = pump.join();
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
