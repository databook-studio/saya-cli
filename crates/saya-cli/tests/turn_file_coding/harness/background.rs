/// A background turn-file run whose pipes are drained on reader threads and
/// whose child is killed on drop: the shape a test needs to signal SIGINT
/// mid-request and still collect both streams (pattern of InteractiveChildGuard
/// in tests/mvp/session.rs).
pub struct Background {
    child: Option<std::process::Child>,
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Drop for Background {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            if child.try_wait().unwrap_or(None).is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

impl Background {
    pub fn id(&self) -> u32 {
        self.child.as_ref().unwrap().id()
    }

    /// Polls the drained stdout buffer until `needle` appears (the child is
    /// provably mid-stream before the test cancels it).
    pub fn wait_for_stdout(&self, needle: &[u8], what: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if self
                .stdout
                .lock()
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle)
            {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("stdout never carried {what}");
    }

    /// Waits for the child to exit; `None` at the deadline.
    pub fn wait_for_exit(&mut self, deadline: Duration) -> Option<std::process::ExitStatus> {
        let start = Instant::now();
        loop {
            if let Some(status) = self.child.as_mut().unwrap().try_wait().unwrap() {
                return Some(status);
            }
            if start.elapsed() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    pub fn stdout(&self) -> String {
        String::from_utf8_lossy(&self.stdout.lock().unwrap()).into_owned()
    }

    pub fn stderr(&self) -> String {
        String::from_utf8_lossy(&self.stderr.lock().unwrap()).into_owned()
    }
}

/// `run` with piped, thread-drained stdout/stderr: the same invocation, but
/// the test stays in control of the child's lifetime and signals.
pub fn run_background(env: &Env, scripted: &Scripted, mode: &str, turn: &str) -> Background {
    let turn_file = env.root.join("turn.txt");
    std::fs::write(&turn_file, turn).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_saya"))
        .args([
            "--workspace",
            env.project.to_str().unwrap(),
            "--turn-file",
            turn_file.to_str().unwrap(),
            "--approval-mode",
            mode,
            "--allow-data-sharing",
            "--format",
            "ndjson",
        ])
        .current_dir(&env.root)
        .env("SAYA_CONFIG_HOME", &env.root)
        .env("SAYA_SESSION_DIR", &env.sessions)
        .env("SAYA_STATE_DB", env.root.join("state.sqlite3"))
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", scripted.address))
        .env("SAYA_API_KEY", "mock-secret")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stdout = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stderr = Arc::new(Mutex::new(Vec::<u8>::new()));
    let stdout_pipe = child.stdout.take().unwrap();
    let stdout_sink = Arc::clone(&stdout);
    thread::spawn(move || {
        let mut reader = stdout_pipe;
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
    let stderr_pipe = child.stderr.take().unwrap();
    let stderr_sink = Arc::clone(&stderr);
    thread::spawn(move || {
        let mut reader = stderr_pipe;
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
    Background {
        child: Some(child),
        stdout,
        stderr,
    }
}

/// Sends SIGINT to the child — the clean cancel channel.
#[cfg(unix)]
pub fn sigint(child_id: u32) {
    assert_eq!(
        unsafe { libc::kill(child_id as libc::pid_t, libc::SIGINT) },
        0,
        "SIGINT was delivered"
    );
}
