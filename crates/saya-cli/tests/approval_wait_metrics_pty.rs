#![cfg(not(windows))]

//! End-to-end ApprovalWait phase proof: a verbose one-shot `ask` under a real
//! PTY records its approval wait sample in the printed local-metrics section
//! when the user answers the prompt, while the automatic path (`read-only`)
//! records none — the section keeps `unavailable`, and the fixed section
//! never carries prompt, SQL, or answer text.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex, Once, mpsc},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(15);

#[test]
fn approval_wait_records_one_sample_when_the_user_answers() {
    warm_up_saya();
    let terminal = run_approval_metrics_case("answered", "ask", Some(b"a\r"));
    assert_metrics_section(&terminal, "phase approval_wait samples=1");
}

#[test]
fn automatic_approval_records_no_approval_wait_sample() {
    warm_up_saya();
    let terminal = run_approval_metrics_case("automatic", "read-only", None);
    assert_metrics_section(&terminal, "phase approval_wait unavailable");
}

/// The shared section assertions: the header printed, the expected
/// approval_wait line, and none of the prompt/SQL/answer sentinels inside
/// the fixed section.
fn assert_metrics_section(terminal: &str, expected: &str) {
    assert!(
        terminal.contains("saya local metrics"),
        "the metrics section prints for a verbose run: {terminal}"
    );
    assert!(
        terminal.contains(expected),
        "expected the approval_wait line `{expected}`: {terminal}"
    );
    let start = terminal.find("saya local metrics").expect("header present");
    let section = &terminal[start..];
    assert!(
        !section.contains("allow once")
            && !section.contains("SELECT 1")
            && !section.contains("done"),
        "the fixed metrics section carries no prompt, SQL, or answer text: {section}"
    );
}

fn warm_up_saya() {
    // The first launch of a freshly linked binary can be slow on macOS (OS
    // scan); pay that one-time cost here, outside the tests' 15s windows.
    static WARM_UP: Once = Once::new();
    WARM_UP.call_once(|| {
        let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_saya"))
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("saya --version spawns");
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(status) = child.try_wait().expect("saya --version is pollable") {
                assert!(status.success(), "saya --version exits successfully");
                return;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                panic!("saya did not start within 120s");
            }
            thread::sleep(Duration::from_millis(50));
        }
    });
}

/// One verbose ask under a real PTY: the mock provider's first response
/// requests the SQL tool; in `ask` mode the harness answers the approval
/// prompt with `a` + CR once raw mode owns the terminal, in every mode the
/// run must settle green with the tool run and its follow-up round, and the
/// printed metrics section returned for the caller's assertions.
fn run_approval_metrics_case(label: &str, approval_mode: &str, answer: Option<&[u8]>) -> String {
    let root = scratch_root(label);
    let _cleanup = RootGuard(root.clone());
    let database = root.join("data.sqlite3");
    fs::write(&database, []).unwrap();
    let config_home = root.join("config-home");
    let saya_config = config_home.join("saya");
    fs::create_dir_all(&saya_config).unwrap();
    fs::write(
        saya_config.join("config.toml"),
        "[ai]\nmodel = 'mock'\nallow_data_sharing = true\n\
         investigation_max_elapsed_seconds = 60\n\n[memory]\nmode = 'off'\n",
    )
    .unwrap();
    let connections = root.join("connections.toml");
    fs::write(
        &connections,
        format!(
            "[profiles.local]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();
    let mut provider = MockProvider::spawn_with_follow_up();
    let pty = native_pty_system();
    let pair = pty
        .openpty(PtySize {
            rows: 24,
            cols: 120,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("native pty allocates");
    let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
    command.cwd(&root);
    command.args([
        "--verbose",
        "--connections",
        connections.to_str().unwrap(),
        "--profile",
        "local",
        "--approval-mode",
        approval_mode,
        "--allow-data-sharing",
        "ask",
        "count rows",
    ]);
    command.env("HOME", &root);
    command.env("SAYA_CONFIG_HOME", &config_home);
    command.env("SAYA_STATE_DB", root.join("state.sqlite3"));
    command.env("SAYA_PROVIDER", "openai_compatible");
    command.env("SAYA_MODEL", "mock");
    command.env("SAYA_PROVIDER_BASE_URL", format!("{}/v1", provider.address));
    command.env("SAYA_API_KEY", "test-only");
    let raw_fd = pair
        .master
        .as_raw_fd()
        .expect("native PTY exposes a file descriptor");
    let mut reader = pair.master.try_clone_reader().expect("pty reader");
    let mut writer = pair.master.take_writer().expect("pty writer");
    let (tx, rx) = std::sync::mpsc::channel();
    let reader_thread = PtyReader::new(thread::spawn(move || {
        let mut bytes = [0_u8; 4096];
        while let Ok(count) = reader.read(&mut bytes) {
            if count == 0 || tx.send(bytes[..count].to_vec()).is_err() {
                break;
            }
        }
    }));
    let mut child = OwnedChild::new(pair.slave.spawn_command(command).expect("saya starts"));
    let started = Instant::now();
    let mut terminal = String::new();
    if let Some(answer) = answer {
        while !terminal.contains("[a] allow once") {
            assert!(
                started.elapsed() < DEADLINE,
                "approval did not become pending: {terminal}"
            );
            if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
                terminal.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        loop {
            let mut termios = unsafe { std::mem::zeroed::<libc::termios>() };
            assert_eq!(
                unsafe { libc::tcgetattr(raw_fd, &mut termios) },
                0,
                "termios reads"
            );
            if termios.c_lflag & libc::ICANON == 0 {
                break;
            }
            assert!(
                started.elapsed() < DEADLINE,
                "the approval prompt printed without its raw terminal owner: {terminal}"
            );
            thread::sleep(Duration::from_millis(10));
        }
        writer
            .write_all(answer)
            .expect("the answer key reaches the raw terminal owner");
    }
    while !terminal.contains("saya local metrics") {
        assert!(
            started.elapsed() < DEADLINE,
            "the metrics section never printed: {terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    while child.try_wait().expect("child is pollable").is_none() {
        assert!(
            started.elapsed() < DEADLINE,
            "the metrics run did not settle: {terminal}"
        );
        if let Ok(bytes) = rx.recv_timeout(Duration::from_millis(100)) {
            terminal.push_str(&String::from_utf8_lossy(&bytes));
        }
    }
    assert_eq!(
        child.try_wait().unwrap().unwrap().exit_code(),
        0,
        "the metrics run succeeds: {terminal}"
    );
    child.disarm();
    assert_eq!(
        provider.requests(),
        2,
        "the tool result returns to the provider exactly once: {terminal}"
    );
    while let Ok(bytes) = rx.recv_timeout(Duration::from_millis(300)) {
        terminal.push_str(&String::from_utf8_lossy(&bytes));
    }
    drop(writer);
    drop(pair);
    reader_thread.join();
    provider.join();
    terminal
}

fn scratch_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-approval-wait-metrics-pty-{label}-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

struct RootGuard(PathBuf);

impl Drop for RootGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct OwnedChild(Option<Box<dyn portable_pty::Child + Send + Sync>>);

impl OwnedChild {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self(Some(child))
    }

    fn try_wait(&mut self) -> std::io::Result<Option<portable_pty::ExitStatus>> {
        self.0
            .as_mut()
            .expect("owned child remains live")
            .try_wait()
    }

    fn disarm(&mut self) {
        self.0.take();
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = self.0.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

struct PtyReader(Option<thread::JoinHandle<()>>);

impl PtyReader {
    fn new(thread: thread::JoinHandle<()>) -> Self {
        Self(Some(thread))
    }

    fn join(mut self) {
        self.0
            .take()
            .expect("PTY reader remains live")
            .join()
            .expect("PTY reader joins after its owner closes");
    }
}

impl Drop for PtyReader {
    fn drop(&mut self) {
        if let Some(thread) = self.0.take() {
            let _ = thread.join();
        }
    }
}

struct MockProvider {
    address: String,
    requests: Arc<Mutex<usize>>,
    stop: mpsc::Sender<()>,
    thread: Option<thread::JoinHandle<()>>,
}

impl MockProvider {
    fn spawn_with_follow_up() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(0));
        let recorded = Arc::clone(&requests);
        let (stop, stopped) = mpsc::channel();
        let thread = thread::spawn(move || {
            listener
                .set_nonblocking(true)
                .expect("provider listener becomes nonblocking");
            let Ok(mut stream) = accept_request(&listener, &stopped) else {
                return;
            };
            drain_request(&mut stream);
            *recorded.lock().unwrap() += 1;
            if stream
                .set_write_timeout(Some(Duration::from_secs(2)))
                .is_err()
            {
                return;
            }
            let response = tool_response();
            if write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response}",
                response.len()
            )
            .is_err()
                || stream.flush().is_err()
            {
                return;
            }
            if let Ok(mut stream) = accept_request(&listener, &stopped) {
                drain_request(&mut stream);
                *recorded.lock().unwrap() += 1;
                if stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .is_ok()
                {
                    let response = answer_response();
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/event-stream\r\nConnection: close\r\n\r\n{response}",
                        response.len()
                    );
                    let _ = stream.flush();
                }
            }
        });
        Self {
            address,
            requests,
            stop,
            thread: Some(thread),
        }
    }

    fn requests(&self) -> usize {
        *self.requests.lock().unwrap()
    }

    fn join(&mut self) {
        let _ = self.stop.send(());
        self.thread
            .take()
            .expect("provider thread joins")
            .join()
            .expect("provider joins cleanly");
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn accept_request(listener: &TcpListener, stop: &mpsc::Receiver<()>) -> std::io::Result<TcpStream> {
    // The mock must outlive a slow child start; the test's overall DEADLINE is the real bound.
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                // Accepted sockets inherit O_NONBLOCK from a non-blocking listener on macOS; drain_request assumes blocking reads under its 2s read timeout.
                stream.set_nonblocking(false)?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if stop.try_recv().is_ok() {
                    return Err(std::io::Error::other("provider stopped"));
                }
                if Instant::now() >= deadline {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "provider request deadline elapsed",
                    ));
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => return Err(error),
        }
    }
}

fn drain_request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("provider request read timeout");
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        if let Some(head) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            let content_length = String::from_utf8_lossy(&bytes[..head])
                .lines()
                .find_map(|line| match line.split_once(':') {
                    Some((name, value)) if name.eq_ignore_ascii_case("content-length") => {
                        value.trim().parse::<usize>().ok()
                    }
                    _ => None,
                })
                .unwrap_or(0);
            if bytes.len() >= head + 4 + content_length {
                return String::from_utf8_lossy(&bytes[head + 4..head + 4 + content_length])
                    .into_owned();
            }
        }
        match stream.read(&mut chunk) {
            Ok(0) => return String::new(),
            Ok(count) => bytes.extend_from_slice(&chunk[..count]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return String::new();
                }
            }
            Err(_) => return String::new(),
        }
    }
}

fn tool_response() -> String {
    let chunk = serde_json::json!({
        "choices": [{"delta": {"tool_calls": [{
            "index": 0,
            "id": "call-1",
            "function": {
                "name": "bounded_sql_query",
                "arguments": "{\"sql\":\"SELECT 1\"}"
            }
        }]}}]
    });
    format!("data: {chunk}\n\ndata: [DONE]\n\n")
}

fn answer_response() -> String {
    "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n".into()
}
