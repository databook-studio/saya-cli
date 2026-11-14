use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

pub(crate) const DEADLINE: Duration = Duration::from_secs(10);

pub(crate) struct ChildGuard(pub(crate) Option<Child>);

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

pub(crate) struct Streams {
    pub(crate) stdout: Arc<Mutex<Vec<u8>>>,
    pub(crate) stderr: Arc<Mutex<Vec<u8>>>,
}

fn drain(buffer: Arc<Mutex<Vec<u8>>>, mut reader: Box<dyn Read + Send>) {
    let mut chunk = [0_u8; 4096];
    while let Ok(size) = reader.read(&mut chunk) {
        if size == 0 {
            break;
        }
        buffer.lock().unwrap().extend_from_slice(&chunk[..size]);
    }
}

pub(crate) fn wait_for(streams: &Streams, needle: &str, label: &str) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        {
            let stdout_guard = streams.stdout.lock().unwrap();
            let stdout = String::from_utf8_lossy(&stdout_guard);
            if stdout.contains(needle) {
                return;
            }
            let stderr_guard = streams.stderr.lock().unwrap();
            let stderr = String::from_utf8_lossy(&stderr_guard);
            if stderr.contains(needle) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "did not observe {label} ({needle})"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

pub(crate) fn scratch_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-reload-repl-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// The user config file the reload re-reads: `SAYA_CONFIG_HOME/saya` is the
/// user config dir, so its `config.toml` is the selected config when no
/// `--config` flag is passed.
pub(crate) fn user_config(root: &std::path::Path) -> std::path::PathBuf {
    root.join("cfg").join("saya").join("config.toml")
}

/// The isolated launch environment: every inherited `SAYA_*` variable is
/// removed, then the ones the test controls are set, so the reload re-reads
/// exactly the files this test writes.
pub(crate) fn isolated(command: &mut Command, root: &std::path::Path) {
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().into_owned();
        if key.starts_with("SAYA_") {
            command.env_remove(&key);
        }
    }
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("APPDATA")
        .env("SAYA_CONFIG_HOME", root.join("cfg"))
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"));
    std::fs::create_dir_all(root.join("sessions")).unwrap();
}

pub(crate) fn spawn_piped(
    root: &std::path::Path,
) -> (ChildGuard, std::process::ChildStdin, Streams) {
    let workspace = root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .arg("--workspace")
        .arg(&workspace)
        .arg("--approval-mode")
        .arg("read-only")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolated(&mut command, root);
    let mut child = command.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let streams = Streams {
        stdout: Arc::new(Mutex::new(Vec::new())),
        stderr: Arc::new(Mutex::new(Vec::new())),
    };
    thread::spawn({
        let sink = Arc::clone(&streams.stdout);
        move || drain(sink, Box::new(stdout))
    });
    thread::spawn({
        let sink = Arc::clone(&streams.stderr);
        move || drain(sink, Box::new(stderr))
    });
    let stdin = child.stdin.take().unwrap();
    (ChildGuard(Some(child)), stdin, streams)
}

pub(crate) fn say(stdin: &mut std::process::ChildStdin, line: &str) {
    use std::io::Write;
    stdin.write_all(format!("{line}\n").as_bytes()).unwrap();
    stdin.flush().unwrap();
}
