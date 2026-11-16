//! One stdio MCP server process, and the bounded exchange this client holds
//! with it.
//!
//! The transport is deliberately blocking and thread-owned rather than a tokio
//! child: a server has to be started from a synchronous composition point (a
//! session's universe, a job's toolset), and the same handle has to serve an
//! async tool call later. One reader thread owns the child's stdout and pushes
//! complete frames into a channel; one drain thread owns stderr and keeps only
//! its bounded tail; the caller writes a frame and waits for the reply that
//! matches its id, under a deadline. A deadline that passes kills the child —
//! which closes the pipe and ends the reader — so no thread and no process
//! outlives a server that stopped answering.
//!
//! The reader never buffers more than [`MAX_FRAME_BYTES`]: a longer line is
//! discarded as it arrives and reported as an oversized frame, whatever the
//! server writes. stderr is drained to `null` beyond its retained tail, so a
//! chatty server cannot fill a pipe and deadlock itself.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::bounds::{MAX_FRAME_BYTES, MAX_SKIPPED_FRAMES, MAX_STDERR_BYTES};

/// One line the reader thread produced.
enum Line {
    /// A complete frame, without its terminator.
    Frame(Vec<u8>),
    /// A line longer than the frame ceiling; its bytes were discarded.
    Oversized,
    /// stdout reached end of file: the server is gone.
    Closed,
}

/// A started server process and the pipes to it.
pub(crate) struct ServerProcess {
    child: Mutex<Child>,
    stdin: Mutex<ChildStdin>,
    lines: Mutex<Receiver<Line>>,
    stderr: Arc<Mutex<Vec<u8>>>,
    /// Serialises a whole request-and-reply exchange. The reader's channel has
    /// one consumer, and one exchange at a time is what makes the id match a
    /// simple equality rather than a table.
    exchange: Mutex<()>,
    /// Whether the process is still believed to be running. Set false by any
    /// transport failure, so a later call fails at once instead of waiting out
    /// another deadline.
    alive: AtomicBool,
}

impl ServerProcess {
    /// Starts `command` with `args` and the entry's own `env`, plus the search
    /// path and nothing else.
    ///
    /// The environment is cleared first, so a credential the operator's shell
    /// happens to hold does not reach a third-party program by accident. `PATH`
    /// is the one exception and it is a deliberate one: most MCP servers are
    /// launched through a package runner (`npx`, `uvx`), and without a search
    /// path the command could not be found at all. `PATH` names where programs
    /// live, not what this process holds.
    pub(crate) fn spawn(
        command: &str,
        args: &[String],
        env: &[(String, String)],
    ) -> Result<Self, String> {
        let mut builder = Command::new(command);
        builder
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(key, value)| (key, value)))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(path) = std::env::var_os("PATH") {
            builder.env("PATH", path);
        }
        let mut child = builder
            .spawn()
            .map_err(|error| format!("could not start {command:?}: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| format!("{command:?} started without a stdin pipe"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| format!("{command:?} started without a stdout pipe"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| format!("{command:?} started without a stderr pipe"))?;
        let (tx, rx) = std::sync::mpsc::channel();
        let retained = Arc::new(Mutex::new(Vec::new()));
        std::thread::spawn(move || read_frames(stdout, tx));
        let sink = Arc::clone(&retained);
        std::thread::spawn(move || drain_stderr(stderr, sink));
        Ok(Self {
            child: Mutex::new(child),
            stdin: Mutex::new(stdin),
            lines: Mutex::new(rx),
            stderr: retained,
            exchange: Mutex::new(()),
            alive: AtomicBool::new(true),
        })
    }

    /// Sends one frame and returns the reply whose id is `id`.
    ///
    /// The whole exchange is one critical section, so a call and a handshake
    /// can never interleave on the same server. Every failure — a write that
    /// cannot complete, an oversized frame, a closed pipe, a deadline that
    /// passes — kills the process and reports what happened; a server that
    /// failed once is never asked again.
    pub(crate) fn exchange(
        &self,
        frame: &[u8],
        id: u64,
        timeout: Duration,
    ) -> Result<Vec<u8>, String> {
        let _serialised = self
            .exchange
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !self.alive.load(Ordering::Acquire) {
            return Err(self.dead_reason());
        }
        let deadline = Instant::now() + timeout;
        if let Err(error) = self.write(frame) {
            return Err(self.fail(error));
        }
        let mut skipped = 0usize;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.fail(format!(
                    "the server did not answer within {} ms",
                    timeout.as_millis()
                )));
            }
            let received = self
                .lines
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .recv_timeout(remaining);
            match received {
                Ok(Line::Frame(bytes)) => match serde_json::from_slice::<serde_json::Value>(&bytes)
                {
                    Ok(value) => {
                        if super::protocol::reply_id(&value) == Some(id) {
                            return Ok(bytes);
                        }
                        // A notification, or a reply to something else: a
                        // server may send either, and neither is this call's
                        // answer.
                        skipped += 1;
                        if skipped > MAX_SKIPPED_FRAMES {
                            return Err(self
                                .fail("the server sent no reply, only other messages".to_owned()));
                        }
                    }
                    Err(_) => {
                        return Err(
                            self.fail("the server sent a frame that is not JSON".to_owned())
                        );
                    }
                },
                Ok(Line::Oversized) => {
                    return Err(self.fail(format!(
                        "the server sent a frame longer than {MAX_FRAME_BYTES} bytes"
                    )));
                }
                Ok(Line::Closed) => {
                    return Err(self.fail("the server closed its output".to_owned()));
                }
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(self.fail("the server's reader stopped".to_owned()));
                }
            }
        }
    }

    /// Writes one frame and flushes it. Public to the client because a
    /// notification has no reply to wait for.
    pub(crate) fn write(&self, frame: &[u8]) -> Result<(), String> {
        let mut stdin = self
            .stdin
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        stdin
            .write_all(frame)
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("could not write to the server: {error}"))
    }

    /// Marks the process dead, kills it, and returns the failure to report.
    /// Reaping is the `Child`'s own: `kill` followed by `wait` leaves no
    /// zombie, and the reader thread ends when the pipe closes.
    fn fail(&self, reason: String) -> String {
        self.kill();
        reason
    }

    /// Kills the process if it is still running. Idempotent: a second call
    /// finds it already dead.
    pub(crate) fn kill(&self) {
        if !self.alive.swap(false, Ordering::AcqRel) {
            return;
        }
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }

    /// The bounded tail of what the server wrote to stderr, for a report.
    pub(crate) fn stderr_tail(&self) -> String {
        let bytes = self
            .stderr
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        String::from_utf8_lossy(&bytes).trim().to_owned()
    }

    /// Why a dead process is dead, as far as this handle can still say.
    fn dead_reason(&self) -> String {
        let tail = self.stderr_tail();
        if tail.is_empty() {
            "the server is no longer running".to_owned()
        } else {
            format!("the server is no longer running; its last output was: {tail}")
        }
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Reads complete frames until the pipe closes. A line longer than the ceiling
/// has its bytes discarded as they arrive and is reported once, so memory
/// stays at the cap whatever the server sends.
fn read_frames(stdout: ChildStdout, tx: Sender<Line>) {
    let mut reader = BufReader::new(stdout);
    loop {
        match read_bounded_line(&mut reader, MAX_FRAME_BYTES) {
            Ok(Some(line)) => {
                if tx.send(Line::Frame(line)).is_err() {
                    return;
                }
            }
            Ok(None) => {
                let _ = tx.send(Line::Closed);
                return;
            }
            Err(LineError::Oversized) => {
                if tx.send(Line::Oversized).is_err() {
                    return;
                }
            }
            Err(LineError::Io) => {
                let _ = tx.send(Line::Closed);
                return;
            }
        }
    }
}

enum LineError {
    /// The line was longer than the cap; it has been discarded whole.
    Oversized,
    /// The pipe failed.
    Io,
}

/// One `\n`-terminated line, capped at `cap` bytes. `Ok(None)` is end of file
/// on a clean boundary. A line at the cap is still delivered: the cap bounds
/// the frame, and a frame of exactly `cap` bytes is one the transport can
/// carry.
fn read_bounded_line<R: BufRead>(reader: &mut R, cap: usize) -> Result<Option<Vec<u8>>, LineError> {
    let mut line = Vec::new();
    let mut oversized = false;
    loop {
        let available = match reader.fill_buf() {
            Ok(bytes) => bytes,
            Err(_) => return Err(LineError::Io),
        };
        if available.is_empty() {
            return if oversized {
                Err(LineError::Oversized)
            } else if line.is_empty() {
                Ok(None)
            } else {
                Ok(Some(line))
            };
        }
        match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => {
                let (head, _) = available.split_at(index);
                if !oversized {
                    if line.len() + head.len() > cap {
                        oversized = true;
                    } else {
                        line.extend_from_slice(head);
                    }
                }
                // Consume the newline as well, so the next call starts on the
                // next line either way.
                reader.consume(index + 1);
                return if oversized {
                    Err(LineError::Oversized)
                } else {
                    Ok(Some(line))
                };
            }
            None => {
                let taken = available.len();
                if !oversized {
                    if line.len() + taken > cap {
                        oversized = true;
                    } else {
                        line.extend_from_slice(available);
                    }
                }
                reader.consume(taken);
            }
        }
    }
}

/// Drains a server's stderr into a bounded tail. The whole stream is read —
/// a full pipe would block the server — and only the last
/// [`MAX_STDERR_BYTES`] are kept.
fn drain_stderr<R: Read>(mut stderr: R, retained: Arc<Mutex<Vec<u8>>>) {
    let mut chunk = [0u8; 4096];
    loop {
        match stderr.read(&mut chunk) {
            Ok(0) | Err(_) => return,
            Ok(read) => {
                let mut tail = retained
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                tail.extend_from_slice(&chunk[..read]);
                let excess = tail.len().saturating_sub(MAX_STDERR_BYTES);
                if excess > 0 {
                    tail.drain(..excess);
                }
            }
        }
    }
}
