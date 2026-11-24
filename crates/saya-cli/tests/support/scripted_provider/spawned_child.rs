// The scripted-provider child: the shared Windows system variables, the output
// pumps, and the guard that reaps it. Included by the harness module, so its
// context is stated as plain comments rather than an inner `//!` header.

include!("../windows_system_env.rs");

/// Waits until `needle` has appeared at least `count` times in `buffer`.
pub(crate) fn wait_for_stream_count(
    buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    needle: &[u8],
    label: &str,
    count: usize,
    timeout: std::time::Duration,
) {
    use std::{thread, time::Instant};

    let deadline = Instant::now() + timeout;
    let needle_owned = needle.to_vec();
    let hits = |locked: &[u8]| {
        locked
            .windows(needle_owned.len().max(1))
            .filter(|window| *window == needle_owned.as_slice())
            .count()
    };
    loop {
        if hits(&buffer.lock().unwrap()) >= count {
            return;
        }
        if Instant::now() >= deadline {
            let full = {
                let locked = buffer.lock().unwrap();
                String::from_utf8_lossy(&locked).into_owned()
            };
            let start = full.len().saturating_sub(2000);
            panic!(
                "did not observe {label} {count}x; buffer ({}/full): {:?}",
                full.len(),
                &full[start..]
            );
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// A spawned REPL child and its captured output. `exit` (or `finish`, which
/// asserts a clean status first) waits for the process, joins the pumps so
/// trailing output is still captured, and removes the temp roots.
pub(crate) struct SpawnedChild {
    pub(crate) guard: InteractiveChildGuard,
    pub(crate) stdout_buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    pub(crate) stderr_buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    /// `None` once `close_stdin` has dropped the pipe: the front then sees EOF.
    stdin: Option<std::process::ChildStdin>,
    config_root: std::path::PathBuf,
    session_root: std::path::PathBuf,
    pumps: Option<(std::thread::JoinHandle<()>, std::thread::JoinHandle<()>)>,
}

impl SpawnedChild {
    pub(crate) fn send(&mut self, bytes: &[u8]) {
        use std::io::Write;
        let stdin = self.stdin.as_mut().expect("stdin is open until it is closed");
        stdin.write_all(bytes).unwrap();
        stdin.flush().unwrap();
    }
    /// Closes the child's stdin. A piped front reads EOF as its quit signal, so
    /// this is the whole exit path for a script under a closed pipe — no
    /// Ctrl-C and no Ctrl-Break, which Windows does not spell the same way.
    #[allow(dead_code)] // only the line front's e2e rows exit on a closed pipe
    pub(crate) fn close_stdin(&mut self) {
        self.stdin = None;
    }
    /// The per-run roots the child was given: goldens redact them before
    /// comparing, so a test may read them without changing what the child
    /// sees. `finish` still removes them.
    pub(crate) fn config_root(&self) -> &std::path::Path {
        &self.config_root
    }
    pub(crate) fn session_root(&self) -> &std::path::Path {
        &self.session_root
    }
    /// Waits for the child to exit, then asserts a clean status.
    pub(crate) fn finish(self) {
        let status = self.exit();
        assert!(status.success(), "child should exit cleanly");
    }
    /// Waits for the child to exit and hands back its own status, so a row can
    /// assert the code (an EOF exit is `0`) rather than only success.
    pub(crate) fn exit(mut self) -> std::process::ExitStatus {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if self.guard.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "interactive child did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        let status = self.guard.0.take().unwrap().wait().unwrap();
        if let Some((err_pump, out_pump)) = self.pumps.take() {
            let _ = err_pump.join();
            let _ = out_pump.join();
        }
        let _ = std::fs::remove_dir_all(&self.config_root);
        let _ = std::fs::remove_dir_all(&self.session_root);
        status
    }
}

/// Spawns the REPL against `base_url` with `ceiling_secs` question budget.
pub(crate) fn spawn_child(base_url: &str, ceiling_secs: u64, tag: &str) -> SpawnedChild {
    spawn_child_with_config(
        base_url,
        &format!("[limits.session]\nelapsed_seconds = {ceiling_secs}\n"),
        tag,
    )
}

/// Spawns the REPL against `base_url` with the caller's full config TOML.
pub(crate) fn spawn_child_with_config(
    base_url: &str,
    config_toml: &str,
    tag: &str,
) -> SpawnedChild {
    spawn_child_with_config_extra(base_url, config_toml, tag, &[])
}

/// The per-run config root `tag` gets. Exposed so a scenario can seed the
/// child's state store before it starts, without duplicating the path rule.
pub(crate) fn config_root_for(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "saya-cli-question-{tag}-{}",
        std::process::id()
    ))
}

/// Spawns the REPL against `base_url` with the caller's full config TOML and
/// any extra CLI arguments (e.g. `--verbose` for the metrics-section rows).
pub(crate) fn spawn_child_with_config_extra(
    base_url: &str,
    config_toml: &str,
    tag: &str,
    extra_args: &[&str],
) -> SpawnedChild {
    use std::{
        io::Read,
        process::{Command, Stdio},
        thread,
    };

    let config_root = config_root_for(tag);
    let session_root = std::env::temp_dir().join(format!(
        "saya-cli-question-sess-{tag}-{}",
        std::process::id()
    ));
    let home_root = config_root.join("home");
    let working_dir = config_root.join("cwd");
    std::fs::create_dir_all(config_root.join("saya")).unwrap();
    std::fs::create_dir_all(&home_root).unwrap();
    std::fs::create_dir_all(&working_dir).unwrap();
    std::fs::write(config_root.join("saya").join("config.toml"), config_toml).unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command.args(extra_args);
    // The session composition reads PATH for the host-command route; the
    // harness clears the environment for hermeticity, so restore the real
    // PATH (Row B passed without it only because nothing it ran needed the
    // route — the startup notice always prints to stderr either way).
    let path = std::env::var_os("PATH").unwrap_or_default();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    // Windows cannot open a socket without its system variables back, so the
    // first request would fail before it reached the mock provider. Nothing
    // else is inherited: Unix keeps exactly the environment it had.
    inherit_windows_system_env(&mut command);
    command
        .env("PATH", path)
        .env("SAYA_CONFIG_HOME", &config_root)
        .env("SAYA_SESSION_DIR", &session_root)
        .env("SAYA_STATE_DB", config_root.join("state.sqlite3"))
        .env("HOME", &home_root)
        .current_dir(&working_dir)
        .env("SAYA_PROVIDER", "openai_compatible")
        .env("SAYA_MODEL", "mock-model")
        .env("SAYA_PROVIDER_BASE_URL", format!("{base_url}/v1"))
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
    let stdin = child.stdin.take().unwrap();
    SpawnedChild {
        guard,
        stdout_buf,
        stderr_buf,
        stdin: Some(stdin),
        config_root,
        session_root,
        pumps: Some((stderr_pump, stdout_pump)),
    }
}
