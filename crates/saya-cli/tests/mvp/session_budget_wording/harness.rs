//! Shared harness for the REPL budget-wording row: the child guard,

#[cfg(unix)]
pub(crate) struct InteractiveChildGuard(pub(crate) Option<std::process::Child>);

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
pub(crate) fn drain_provider_request(stream: &mut std::net::TcpStream) -> String {
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
pub(crate) fn accept_provider(listener: &std::net::TcpListener) -> Option<std::net::TcpStream> {
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
pub(crate) fn wait_for_stream_timeout(
    buffer: &std::sync::Arc<std::sync::Mutex<Vec<u8>>>,
    needle: &[u8],
    label: &str,
    timeout: std::time::Duration,
) {
    use std::{thread, time::Instant};

    let deadline = Instant::now() + timeout;
    let needle_owned = needle.to_vec();
    loop {
        if buffer
            .lock()
            .unwrap()
            .windows(needle_owned.len())
            .any(|window| window == needle_owned.as_slice())
        {
            return;
        }
        if Instant::now() >= deadline {
            let full = {
                let locked = buffer.lock().unwrap();
                String::from_utf8_lossy(&locked).into_owned()
            };
            let start = full.len().saturating_sub(2000);
            panic!(
                "did not observe {label}; buffer ({}/full): {:?}",
                full.len(),
                &full[start..]
            );
        }
        thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// One reply the scriptable mock server sends per provider request.
#[cfg(unix)]
#[derive(Clone)]
pub(crate) enum ScriptStep {
    Harmless,
    HoldOpen { marker: String },
}
#[cfg(unix)]
pub(crate) fn harmless_body() -> String {
    "data: {\"content\":\"{\\\"proposals\\\": []}\"}\n\n".to_owned()
}

#[cfg(unix)]
fn reply_finished(stream: &mut std::net::TcpStream, body: &str) {
    use std::io::Write;
    let done = format!("data: [{}]\n\n", "DONE");
    let full = format!("{body}{done}");
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{full}"
    )
    .unwrap();
    stream.flush().unwrap();
}

#[cfg(unix)]
pub(crate) struct MockServer {
    pub(crate) address: String,
    pub(crate) requests: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(unix)]
impl MockServer {
    pub(crate) fn join(&mut self) {
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

#[cfg(unix)]
impl Drop for MockServer {
    fn drop(&mut self) {
        self.join();
    }
}

/// Spawns the mock provider thread: request `i` is answered by
/// `script[i]` (requests past the script get a harmless completion).
/// Clarify holds `delay` inside the thread before writing (source-held
/// ACTIVE time); HoldOpen writes one marker chunk and keeps the socket
/// open until `release` fires, so no new request can arrive meanwhile.
#[cfg(unix)]
pub(crate) fn spawn_script_server(
    listener: std::net::TcpListener,
    script: Vec<ScriptStep>,
    release: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> MockServer {
    use std::{io::Write, sync::atomic::Ordering, thread, time::Duration};

    let address = format!("http://{}", listener.local_addr().unwrap());
    let requests = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let server_count = requests.clone();
    let handle = thread::spawn(move || {
        loop {
            let Some(mut stream) = accept_provider(&listener) else {
                break;
            };
            drain_provider_request(&mut stream);
            let index = server_count.fetch_add(1, Ordering::SeqCst);
            let step = script.get(index).cloned().unwrap_or(ScriptStep::Harmless);
            match step {
                ScriptStep::Harmless => reply_finished(&mut stream, &harmless_body()),
                ScriptStep::HoldOpen { marker } => {
                    // A 429 establishment refusal: the loop backs off and
                    // retries the SAME answering request against the next
                    // script slot, so the run stays inside the answering
                    // source, held, until the remaining budget fires. (A
                    // 200 + content + DONE would settle the turn naturally
                    // instead of tripping the ceiling.) `marker` selects
                    // which unfinished chunk the retry serves: empty keeps
                    // the socket silent; non-empty writes one content chunk
                    // (already on screen, proving the reply turn runs)
                    // with no DONE frame, then holds the socket open.
                    if marker.is_empty() {
                        // Hold the socket silent: accept the request, say
                        // nothing, keep it open. The establishment (60s)
                        // and per-chunk idle (90s) timeouts never fire, so
                        // the runtime's own remaining-elapsed timer is what
                        // ends the reply turn — still ACTIVE elapsed time
                        // of the same question scope. (A 429 refusal is NOT
                        // usable here: a body on a 429 confuses reqwest's
                        // framing and the loop classes it as a not-retried
                        // provider failure instead of backing off.)
                        use std::time::Instant;
                        let held = Instant::now();
                        while !release.load(Ordering::SeqCst) {
                            if held.elapsed() > Duration::from_secs(25) {
                                break;
                            }
                            thread::sleep(Duration::from_millis(20));
                        }
                        return;
                    }
                    let head = "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\r\n";
                    write!(stream, "{head}data: {marker}\n\n").unwrap();
                    stream.flush().unwrap();
                    use std::time::Instant;
                    let held = Instant::now();
                    let mut beats = 0_u32;
                    while !release.load(Ordering::SeqCst) {
                        if held.elapsed() > Duration::from_secs(25) {
                            break;
                        }
                        if beats.is_multiple_of(10) {
                            let _ = write!(stream, ": keep-alive\n\n");
                            let _ = stream.flush();
                        }
                        beats = beats.wrapping_add(1);
                        thread::sleep(Duration::from_millis(20));
                    }
                    return;
                }
            }
            drop(stream);
            // The latch rows script up to ten requests (fail, /compact
            // re-arm, /clear re-arm); bound runaway loops well past that.
            if index >= 16 {
                break;
            }
        }
    });
    MockServer {
        address,
        requests,
        handle: Some(handle),
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
