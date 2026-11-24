//! The full-screen front on a pseudo-terminal at the plan's 100x30, with its
//! byte stream captured raw.
//!
//! The raw stream is what a frame count has to come from: ratatui's crossterm
//! backend emits one cursor-visibility command per `Terminal::draw` — `Hide`
//! at the start of a frame whose cursor is elsewhere, `Show` at the end of one
//! that draws it — and no other part of the surface writes those sequences. So
//! the count of both markers between two instants is the number of frames
//! painted between them, with no production instrumentation at all.

use std::{
    io::{Read, Write},
    path::Path,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

use crate::spawn::{self, Roots};

/// The surface's own size for every draw measure, as the plan states it.
pub(crate) const COLS: u16 = 100;
pub(crate) const ROWS: u16 = 30;
/// `Hide` and `Show`: the two commands ratatui queues once per frame.
pub(crate) const FRAME_MARKERS: [&[u8]; 2] = [b"\x1b[?25l", b"\x1b[?25h"];

/// A pty read that has not answered yet: short, so the drain loop can check its
/// own deadline even when the surface has gone quiet.
const READ_TIMEOUT: Duration = Duration::from_millis(40);

pub(crate) struct TuiChild {
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send>,
    rx: mpsc::Receiver<std::io::Result<Vec<u8>>>,
    /// Every byte the surface has written, in arrival order.
    raw: Arc<Mutex<Vec<u8>>>,
    parser: vt100::Parser,
    pub(crate) pid: u32,
    roots: Roots,
}

impl TuiChild {
    /// Spawns `bin` on a fresh pty with the run's config and environment.
    pub(crate) fn spawn(
        bin: &Path,
        base_url: &str,
        roots: Roots,
        config: &str,
        args: &[String],
    ) -> Self {
        let env = spawn::prepare(&roots, config, base_url);
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: ROWS,
                cols: COLS,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(bin.to_str().unwrap());
        command.env_clear();
        for (key, value) in &env {
            command.env(key, value);
        }
        command.args(args.iter().map(String::as_str));
        command.cwd(roots.config_root.join("cwd"));
        let child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let writer = pair.master.take_writer().unwrap();
        let mut reader = pair.master.try_clone_reader().unwrap();
        let raw = Arc::new(Mutex::new(Vec::new()));
        let sink = raw.clone();
        let (tx, rx) = mpsc::channel();
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(size) => {
                        sink.lock().unwrap().extend_from_slice(&buffer[..size]);
                        if tx.send(Ok(buffer[..size].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        let pid = child.process_id().expect("the surface's own pid");
        Self {
            writer,
            child,
            rx,
            raw,
            parser: vt100::Parser::new(ROWS, COLS, 0),
            pid,
            roots,
        }
    }

    /// Types one line and returns the instant the write started.
    pub(crate) fn send_line(&mut self, line: &str) -> Instant {
        let start = Instant::now();
        let _ = self.writer.write_all(format!("{line}\r").as_bytes());
        let _ = self.writer.flush();
        start
    }

    /// The text the surface is showing, as a terminal would show it.
    pub(crate) fn screen(&self) -> String {
        let screen = self.parser.screen();
        let mut text = String::new();
        for row in screen.rows(0, COLS) {
            text.push_str(row.trim_end());
            text.push('\n');
        }
        text
    }

    /// How many bytes the surface has written so far.
    pub(crate) fn raw_len(&self) -> usize {
        self.raw.lock().unwrap().len()
    }

    /// Every byte the surface wrote from `from` on.
    pub(crate) fn raw_since(&self, from: usize) -> Vec<u8> {
        let raw = self.raw.lock().unwrap();
        raw.get(from..).unwrap_or_default().to_vec()
    }

    /// Drains one read, if the surface has said anything.
    fn pump(&mut self) -> Result<bool, String> {
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) => {
                self.parser.process(&bytes);
                Ok(true)
            }
            Ok(Err(error)) => Err(format!("pty read failed: {error}")),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(format!("the surface exited; screen:\n{}", self.screen()))
            }
        }
    }

    /// Drains the surface until `deadline`, so a long window never blocks it.
    pub(crate) fn drain_until(&mut self, deadline: Instant) -> Result<(), String> {
        while Instant::now() < deadline {
            self.pump()?;
        }
        Ok(())
    }

    /// Drains until the screen stops changing for `quiet`, or `within` is up.
    pub(crate) fn settle(&mut self, quiet: Duration, within: Duration) -> Result<(), String> {
        let deadline = Instant::now() + within;
        let mut last = String::new();
        let mut stable_at = Instant::now();
        loop {
            self.pump()?;
            let now = Instant::now();
            let screen = self.screen();
            if screen != last {
                last = screen;
                stable_at = now;
            } else if now.duration_since(stable_at) >= quiet {
                return Ok(());
            }
            if now >= deadline {
                return Ok(());
            }
        }
    }

    /// Drains until the screen or the raw stream shows `needle`: a long
    /// streamed answer may scroll the words it ends with off the visible
    /// screen before the drain catches them.
    pub(crate) fn wait_for_output(&mut self, needle: &str, within: Duration) -> Result<(), String> {
        let deadline = Instant::now() + within;
        loop {
            self.pump()?;
            let said = self
                .raw
                .lock()
                .unwrap()
                .windows(needle.len())
                .any(|window| window == needle.as_bytes());
            if said || self.screen().contains(needle) {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "'{needle}' never appeared within {within:?}; screen:\n{}",
                    self.screen()
                ));
            }
        }
    }
}

impl Drop for TuiChild {
    fn drop(&mut self) {
        let _ = self.writer.write_all(b"\x03");
        let _ = self.writer.flush();
        let _ = self.child.kill();
        let _ = std::fs::remove_dir_all(&self.roots.config_root);
        let _ = std::fs::remove_dir_all(&self.roots.session_root);
    }
}
