//! The approval panel's terminal harness: a private checkout, a provider that
//! answers, and the real binary on a real pty at a chosen size.
//!
//! Nothing here decides what the surface says. It spawns, it feeds keys, and it
//! reports the screen the terminal came out with (`vt100`, the same parser the
//! rest of the pty suite reads).

use super::{IO_WAIT, OUTPUT_LIMIT, PAINT, SETTLE};
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::{
    fs,
    io::{Read, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

/// The screen one case runs on: the panel's two review sizes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Size {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// 100x30, the design's wide screen.
pub(crate) const WIDE: Size = Size {
    cols: 100,
    rows: 30,
};

/// 60x24, the narrow screen the panel must stay readable on.
pub(crate) const NARROW: Size = Size { cols: 60, rows: 24 };

/// The scratch folder a case owns, removed when it is dropped.
struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// One launch of the surface on a pty.
pub(crate) struct Panel {
    _root: Root,
    child: Option<Box<dyn portable_pty::Child + Send + Sync>>,
    writer: Box<dyn Write + Send>,
    output: std::sync::mpsc::Receiver<Vec<u8>>,
    parser: vt100::Parser,
    raw: Vec<u8>,
    _provider: Provider,
}

impl Panel {
    /// Opens the surface at `size` in its own checkout, pointed at a provider
    /// that answers, under `ask` so an effect is put to the operator.
    pub(crate) fn open(label: &str, size: Size) -> Self {
        let root = std::env::temp_dir().join(format!(
            "saya-approval-panel-{label}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let home = root.join("home");
        fs::create_dir_all(home.join("saya")).expect("the scratch config home");
        let repo = root.join("repo");
        fs::create_dir_all(&repo).expect("the scratch checkout");
        git(&repo);
        let provider = Provider::spawn();
        fs::write(
            home.join("saya/config.toml"),
            format!(
                "[ai]\nprovider = 'openai_compatible'\nbase_url = '{}'\nmodel = 'pty-fixture'\n\
                 [limits.session]\nelapsed_seconds = 120\n",
                provider.endpoint()
            ),
        )
        .expect("the scratch config");
        fs::write(
            home.join("saya/connections.toml"),
            "[profiles.analytics]\ntype = 'sqlite'\npath = 'analytics.sqlite3'\n",
        )
        .expect("the scratch connections");
        fs::write(repo.join("analytics.sqlite3"), []).expect("the scratch database");

        let pair = native_pty_system()
            .openpty(PtySize {
                rows: size.rows,
                cols: size.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("a native pty allocates");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
        command.args([
            "--workspace",
            repo.to_str().unwrap(),
            "--profile",
            "analytics",
            "--approval-mode",
            "ask",
        ]);
        command.cwd(&repo);
        command.env("HOME", &home);
        command.env("SAYA_CONFIG_HOME", &home);
        command.env("SAYA_STATE_DB", home.join("state.sqlite3"));
        command.env_remove("SAYA_API_KEY");
        command.env_remove("SAYA_AI_API_KEY");
        let child = pair
            .slave
            .spawn_command(command)
            .expect("the surface starts on the pty");
        drop(pair.slave);
        let writer = pair.master.take_writer().expect("the pty writer");
        let reader = pair.master.try_clone_reader().expect("the pty reader");
        let (tx, output) = std::sync::mpsc::channel();
        thread::spawn(move || drain(reader, tx));
        Self {
            _root: Root(root),
            child: Some(child),
            writer,
            output,
            parser: vt100::Parser::new(size.rows, size.cols, 0),
            raw: Vec::new(),
            _provider: provider,
        }
    }

    /// The screen as the operator would read it.
    pub(crate) fn screen(&self) -> String {
        rows(self.parser.screen())
    }

    /// Types one line and presses Enter.
    pub(crate) fn send_line(&mut self, line: &str) {
        self.send(line.as_bytes());
        self.send(b"\r");
    }

    /// Types raw bytes: a key, or a line without its Enter.
    pub(crate) fn send(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .expect("the bytes reach the pty");
        self.writer.flush().expect("the bytes are flushed");
    }

    /// Drains the pty until the screen has stopped changing: a frame is written
    /// in more than one chunk, and a capture taken between them would show half
    /// a panel. What a test reads is the settled screen an operator sees.
    pub(crate) fn settle(&mut self) {
        let mut previous = self.screen();
        let mut stable = Instant::now();
        let started = Instant::now();
        while stable.elapsed() < SETTLE && started.elapsed() < PAINT {
            self.pump();
            let now = self.screen();
            if now != previous {
                previous = now;
                stable = Instant::now();
            }
        }
        assert!(
            started.elapsed() < PAINT,
            "the screen never settled; screen:\n{previous}"
        );
    }

    /// Waits until `needle` is on screen, draining the pty as it goes.
    pub(crate) fn wait_for(&mut self, needle: &str, within: Duration) {
        let started = Instant::now();
        while !self.screen().contains(needle) {
            assert!(
                started.elapsed() < within,
                "the screen never showed {needle:?} within {within:?}; screen:\n{}",
                self.screen()
            );
            self.pump();
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.try_wait();
            let _ = child.kill();
            let _ = child.try_wait();
        }
    }
}

impl Panel {
    fn pump(&mut self) {
        match self.output.recv_timeout(IO_WAIT) {
            Ok(bytes) if bytes.is_empty() => {}
            Ok(bytes) => {
                self.raw.extend_from_slice(&bytes);
                assert!(
                    self.raw.len() <= OUTPUT_LIMIT,
                    "the pty transcript is bounded"
                );
                self.parser.process(&bytes);
            }
            Err(_) => {}
        }
    }
}

/// The screen's text, one row per line, trailing blanks trimmed: the rows the
/// terminal holds, so a cell the surface never wrote reads as a blank.
fn rows(screen: &vt100::Screen) -> String {
    let (_, width) = screen.size();
    let mut text = String::new();
    for row in screen.rows(0, width) {
        text.push_str(row.trim_end());
        text.push('\n');
    }
    text
}

/// A checkout the launch can bind as its workspace: one committed file.
fn git(repo: &PathBuf) {
    for args in [
        ["init", "--quiet"].as_slice(),
        ["config", "user.name", "Saya Panel Fixture"].as_slice(),
        ["config", "user.email", "panel@example.invalid"].as_slice(),
        ["commit", "--quiet", "--allow-empty", "-m", "baseline"].as_slice(),
    ] {
        let status = std::process::Command::new("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git runs");
        assert!(status.success(), "git {args:?} succeeds");
    }
}

/// Drains the pty onto a channel until the child closes it.
fn drain(mut reader: Box<dyn Read + Send>, tx: std::sync::mpsc::Sender<Vec<u8>>) {
    let mut buffer = [0_u8; 4096];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(count) => {
                if tx.send(buffer[..count].to_vec()).is_err() {
                    break;
                }
            }
        }
    }
}

/// A provider that answers every request with one sentence: the launch has
/// somewhere to talk to, and no test here waits on a model.
struct Provider {
    address: String,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _requests: Arc<Mutex<u64>>,
}

impl Provider {
    fn spawn() -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("the mock provider binds");
        listener
            .set_nonblocking(true)
            .expect("the listener becomes nonblocking");
        let address = format!("http://{}", listener.local_addr().expect("its address"));
        let stop = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(0_u64));
        let stopped = Arc::clone(&stop);
        let counted = Arc::clone(&requests);
        let worker = thread::spawn(move || {
            while !stopped.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        *counted.lock().expect("the counter") += 1;
                        let body = "data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"one answer\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.flush();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            address,
            stop,
            worker: Some(worker),
            _requests: requests,
        }
    }

    fn endpoint(&self) -> String {
        format!("{}/v1", self.address)
    }
}

impl Drop for Provider {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take();
    }
}
