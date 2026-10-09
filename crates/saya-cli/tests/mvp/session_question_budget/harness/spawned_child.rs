/// Waits until `needle` has appeared at least `count` times in `buffer`.
#[cfg(unix)]
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

/// Waits for the child to exit, then asserts a clean status and removes
/// the temp config/session roots. Pumps are joined after the process ends
/// so trailing output is still captured.
#[cfg(unix)]
pub(crate) struct SpawnedChild {
    pub(crate) guard: InteractiveChildGuard,
    pub(crate) stdout_buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    pub(crate) stderr_buf: std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    pub(crate) stdin: std::process::ChildStdin,
    config_root: std::path::PathBuf,
    session_root: std::path::PathBuf,
    pumps: Option<(std::thread::JoinHandle<()>, std::thread::JoinHandle<()>)>,
}

#[cfg(unix)]
impl SpawnedChild {
    pub(crate) fn send(&mut self, bytes: &[u8]) {
        use std::io::Write;
        self.stdin.write_all(bytes).unwrap();
        self.stdin.flush().unwrap();
    }
    pub(crate) fn finish(mut self) {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if self.guard.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "interactive child did not exit");
            std::thread::sleep(Duration::from_millis(20));
        }
        let status = self.guard.0.take().unwrap().wait().unwrap();
        assert!(status.success(), "child should exit cleanly");
        if let Some((err_pump, out_pump)) = self.pumps.take() {
            let _ = err_pump.join();
            let _ = out_pump.join();
        }
        let _ = std::fs::remove_dir_all(&self.config_root);
        let _ = std::fs::remove_dir_all(&self.session_root);
    }
}

/// Spawns the REPL against `base_url` with `ceiling_secs` question budget.
#[cfg(unix)]
pub(crate) fn spawn_child(base_url: &str, ceiling_secs: u64, tag: &str) -> SpawnedChild {
    spawn_child_with_config(
        base_url,
        &format!("[ai]\ninvestigation_max_elapsed_seconds = {ceiling_secs}\n"),
        tag,
    )
}

/// Spawns the REPL against `base_url` with the caller's full config TOML.
#[cfg(unix)]
pub(crate) fn spawn_child_with_config(
    base_url: &str,
    config_toml: &str,
    tag: &str,
) -> SpawnedChild {
    spawn_child_with_config_extra(base_url, config_toml, tag, &[])
}

/// Spawns the REPL against `base_url` with the caller's full config TOML and
/// any extra CLI arguments (e.g. `--verbose` for the metrics-section rows).
#[cfg(unix)]
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

    let pid = std::process::id();
    let config_root = std::env::temp_dir().join(format!("saya-cli-question-{tag}-{pid}"));
    let session_root = std::env::temp_dir().join(format!("saya-cli-question-sess-{tag}-{pid}"));
    std::fs::create_dir_all(config_root.join("saya")).unwrap();
    std::fs::write(config_root.join("saya").join("config.toml"), config_toml).unwrap();

    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command.args(extra_args);
    // The session composition reads PATH for the host-command lane; the
    // harness clears the environment for hermeticity, so restore the real
    // PATH (Row B passed without it only because nothing it ran needed the
    // lane — the startup notice always prints to stderr either way).
    let path = std::env::var_os("PATH").unwrap_or_default();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear()
        .env("PATH", path)
        .env("SAYA_CONFIG_HOME", &config_root)
        .env("SAYA_SESSION_DIR", &session_root)
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
        stdin,
        config_root,
        session_root,
        pumps: Some((stderr_pump, stdout_pump)),
    }
}
