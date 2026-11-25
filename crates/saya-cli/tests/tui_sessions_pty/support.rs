//! One fixture and one launch for the picker's pty case.
//!
//! The scratch home holds the config the surface opens with, a project folder,
//! and two saved sessions whose titles an operator chooses between — the newer
//! one is the row `↵` lands on, because the store lists newest first.

#![allow(dead_code)] // shared test harness: this target uses part of it

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use saya_cli::SessionState;
use saya_store::{FsSessionStore, SessionStore};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

include!("../support/provider_env.rs");

/// The older record's first question: the title its row carries.
pub(crate) const OLDER: &str = "the older question";
/// The newer record's first question, and the row `↵` chooses.
pub(crate) const NEWER: &str = "the newer question";
const COLS: u16 = 100;
const ROWS: u16 = 30;
const READ_TIMEOUT: Duration = Duration::from_millis(250);
const DEADLINE: Duration = Duration::from_secs(30);

/// A launched child, its pty's screen, and the home it was given: dropped
/// together, so a failing assertion leaves no process and no fixture behind.
pub(crate) struct Session {
    home: PathBuf,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<std::io::Result<Vec<u8>>>,
    parser: vt100::Parser,
    /// The width the frame is read at: the pty's own, stated when it was made.
    cols: u16,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.try_wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Session {
    /// Types one line into the surface and presses Enter.
    pub(crate) fn send_line(&mut self, line: &str) {
        self.writer
            .write_all(format!("{line}\r").as_bytes())
            .expect("the pty accepts a line");
        self.writer.flush().expect("the pty flushes the line");
    }

    /// Presses one key: the plain Enter, or Esc.
    pub(crate) fn press(&mut self, bytes: &[u8]) {
        self.writer
            .write_all(bytes)
            .expect("the pty accepts the press");
        self.writer.flush().expect("the pty flushes the press");
    }

    /// Waits until the screen holds `needle`, and answers the whole screen.
    pub(crate) fn wait_for(&mut self, needle: &str) -> String {
        let start = Instant::now();
        loop {
            let screen = self.screen();
            if screen.contains(needle) {
                return screen;
            }
            assert!(
                start.elapsed() < DEADLINE,
                "never saw {needle:?}; screen:\n{screen}"
            );
            match self.rx.recv_timeout(READ_TIMEOUT) {
                Ok(Ok(bytes)) if bytes.is_empty() => panic!(
                    "the child exited before {needle:?} appeared; screen:\n{}",
                    self.screen()
                ),
                Ok(Ok(bytes)) => self.parser.process(&bytes),
                Ok(Err(error)) => panic!("pty read failed: {error}"),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("pty reader ended"),
            }
        }
    }

    /// Reads whatever the surface has painted for `for_`, and answers nothing:
    /// the frame a caller reads next is the one that settled in that time.
    pub(crate) fn settle(&mut self, for_: Duration) {
        let start = Instant::now();
        while start.elapsed() < for_ {
            match self.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(Ok(bytes)) if !bytes.is_empty() => self.parser.process(&bytes),
                Ok(Ok(_)) => {}
                Ok(Err(_)) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            }
        }
    }

    /// The settled frame, one line per terminal row.
    pub(crate) fn screen(&self) -> String {
        let mut out = String::new();
        for row in self.parser.screen().rows(0, self.cols) {
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }
}

/// The scratch home the case launches over, with its two saved sessions.
///
/// It lives under the platform's temp directory rather than beside the build: the
/// launch folder is inside this repository, so the workspace root is, and the
/// product refuses a state root inside the workspace it serves (`resume_posture_pty`
/// keeps its home outside for the same reason). The home is removed when the case
/// ends.
pub(crate) fn fixture(tag: &str) -> PathBuf {
    let home = std::env::temp_dir().join(format!("saya-tui-sessions-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the scratch home builds");

    let config = home.join("config-home").join("saya");
    std::fs::create_dir_all(&config).expect("the user config dir builds");
    std::fs::write(
        config.join("config.toml"),
        "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n",
    )
    .expect("the config writes");

    let project = home.join("project");
    std::fs::create_dir_all(&project).expect("the project dir builds");
    seed(&home, "20260101-000000-older00", OLDER, "older answer");
    seed(&home, "20260102-000000-newer00", NEWER, "newer answer");
    home
}

/// Saves one record: two turns, pinned to the fixture's project folder, so a
/// resume finds a folder it may open.
fn seed(home: &Path, id: &str, question: &str, answer: &str) {
    let store = FsSessionStore::new(home.join("sessions"));
    let mut state = SessionState::new(id, None, "smoke");
    state.workspace_root = Some(home.join("project").display().to_string());
    state.record_turn(question, answer, false, Vec::new());
    state.record_turn("a later question", "a later answer", false, Vec::new());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test tokio runtime builds")
        .block_on(store.save(state.redacted()))
        .expect("the record saves");
}

/// Launches the real binary over `home`, in a launch folder of its own, on the
/// size the picker cases read.
pub(crate) fn launch(home: &Path) -> Session {
    launch_at(home, COLS, ROWS)
}

/// The same launch at a size a caller states: a pane is read as the width it
/// covers, so a case about a narrow terminal states its own.
pub(crate) fn launch_at(home: &Path, cols: u16, rows: u16) -> Session {
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows,
        cols,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(reason) => panic!("could not allocate a pty: {reason}"),
    };
    let mut cmd = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_saya")));
    cmd.env("HOME", home);
    cmd.env("SAYA_CONFIG_HOME", home.join("config-home"));
    cmd.env("SAYA_SESSION_DIR", home.join("sessions"));
    cmd.env("SAYA_STATE_DB", home.join("state.sqlite3"));
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    cmd.env_remove("SAYA_API_KEY");
    cmd.env_remove("SAYA_AI_API_KEY");
    // The fixture states no provider: no key from the host may choose one.
    for name in PROVIDER_ENV {
        cmd.env_remove(name);
    }
    let launch_dir = home.join("launch-dir");
    std::fs::create_dir_all(&launch_dir).expect("the launch dir builds");
    cmd.cwd(launch_dir);
    let child = pair.slave.spawn_command(cmd).expect("the binary spawns");
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .expect("the pty reader clones");
    let writer = pair.master.take_writer().expect("the pty writer is taken");
    Session {
        home: home.to_path_buf(),
        writer,
        child,
        rx: reader_channel(reader),
        parser: vt100::Parser::new(rows, cols, 0),
        cols,
    }
}

fn reader_channel(reader: Box<dyn Read + Send>) -> Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0_u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    let _ = tx.send(Ok(Vec::new()));
                    break;
                }
                Ok(n) => {
                    if tx.send(Ok(buf[..n].to_vec())).is_err() {
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
    rx
}
