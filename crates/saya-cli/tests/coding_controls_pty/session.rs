use super::DEADLINE;
use portable_pty::Child;
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::mpsc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[path = "session/drive.rs"]
mod drive;
#[path = "session/spawn.rs"]
mod spawn;

const OUTPUT_LIMIT: usize = 256 * 1024;
const INPUT_LIMIT: usize = 32 * 1024;
const IO_WAIT: Duration = Duration::from_millis(100);
pub(super) const PTY_ROWS: u16 = 30;
pub(super) const PTY_COLS: u16 = 120;

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub struct Session {
    root: Root,
    child: Option<Box<dyn Child + Send + Sync>>,
    writer: Option<Box<dyn Write + Send>>,
    output: mpsc::Receiver<Vec<u8>>,
    reader: Option<JoinHandle<()>>,
    parser: vt100::Parser,
    raw: Vec<u8>,
    child_reaped: bool,
    reader_finished: bool,
}

impl Session {
    pub fn finish(&mut self) -> Vec<u8> {
        let deadline = Instant::now() + DEADLINE;
        while Instant::now() < deadline {
            // A quit can be deferred while the coding panel settles its job
            // ("a coding job is still running"); re-arming and quitting again
            // is idempotent, so keep offering the quit until the child exits.
            self.send(b"\x03\x03");
            let until = Instant::now() + Duration::from_millis(250);
            while Instant::now() < until {
                if self
                    .child
                    .as_mut()
                    .and_then(|child| child.try_wait().ok())
                    .flatten()
                    .is_some()
                {
                    break;
                }
                self.pump();
            }
            if self
                .child
                .as_mut()
                .and_then(|child| child.try_wait().ok())
                .flatten()
                .is_some()
            {
                break;
            }
        }
        self.cleanup(Duration::from_secs(2));
        self.raw.clone()
    }

    pub fn records(&self, job: &str) -> String {
        let root = self.root.0.join("home/coding").join(job);
        let mut output = Vec::new();
        let entries = fs::read_dir(root).expect("owned coding records directory exists");
        for entry in entries.take(128) {
            let entry = entry.unwrap();
            if !entry.file_type().unwrap().is_file() {
                continue;
            }
            let bytes = fs::read(entry.path()).unwrap();
            if bytes.len() <= 1024 * 1024 {
                output.extend_from_slice(&bytes);
            }
            assert!(output.len() <= 4 * 1024 * 1024, "records are bounded");
        }
        String::from_utf8_lossy(&output).into_owned()
    }

    pub fn state_db(&self) -> PathBuf {
        self.root.0.join("home/state.sqlite3")
    }

    pub fn screen_text(&self) -> String {
        let mut text = String::new();
        for row in self.parser.screen().rows(0, PTY_COLS) {
            text.push_str(row.trim_end());
            text.push('\n');
        }
        text
    }

    pub fn child_reaped(&self) -> bool {
        self.child_reaped
    }

    pub fn reader_finished(&self) -> bool {
        self.reader_finished
    }

    fn cleanup(&mut self, drain_for: Duration) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let deadline = Instant::now() + drain_for;
            while Instant::now() < deadline {
                match child.try_wait() {
                    Ok(Some(_)) => {
                        self.child_reaped = true;
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(10)),
                    Err(_) => break,
                }
            }
        }
        self.child = None;
        self.writer.take();
        let deadline = Instant::now() + drain_for;
        let mut reader_finished = false;
        while Instant::now() < deadline {
            match self.output.recv_timeout(IO_WAIT) {
                Ok(chunk) => {
                    self.raw.extend_from_slice(&chunk);
                    self.parser.process(&chunk);
                    if self.raw.len() >= OUTPUT_LIMIT {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    reader_finished = true;
                    break;
                }
            }
        }
        if reader_finished {
            self.reader_finished = true;
            if let Some(reader) = self.reader.take() {
                let _ = reader.join();
            }
        } else {
            if self.reader.as_ref().is_some_and(JoinHandle::is_finished) {
                self.reader_finished = true;
                if let Some(reader) = self.reader.take() {
                    let _ = reader.join();
                }
            } else {
                self.reader.take();
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cleanup(Duration::from_millis(100));
    }
}
