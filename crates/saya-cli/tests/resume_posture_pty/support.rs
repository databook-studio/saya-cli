//! The pty harness both resume cases drive.
//!
//! One scratch home, one fixture and one launch: a config whose user-layer deny
//! list names `curl`, a profile over an empty database file, a project folder,
//! and a saved session record — a record with no run link, which is what a 0.5.0
//! session's save looks like. A case states only the launch and the words it
//! waits for.

#![allow(dead_code)]

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use saya_cli::SessionState;
use saya_store::{FsSessionStore, SessionStore};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

/// The session both cases resume: a 0.5.0 record, saved with no run link.
pub(crate) const RESUMED_ID: &str = "20260101-000000-pty0";
const COLS: u16 = 80;
const ROWS: u16 = 24;
const READ_TIMEOUT: Duration = Duration::from_millis(250);
const DEADLINE: Duration = Duration::from_secs(30);

/// A launched child, its pty's screen, and the home it was given: dropped
/// together, so a failing assertion leaves no process and no fixture behind.
pub(crate) struct Session {
    home: PathBuf,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    rx: Receiver<std::io::Result<Vec<u8>>>,
    parser: vt100::Parser,
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.try_wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

impl Session {
    /// Waits until the settled frame holds `needle`, and answers the whole
    /// screen so a case can assert on more of it.
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

    /// The settled frame, one line per terminal row.
    pub(crate) fn screen(&self) -> String {
        let mut out = String::new();
        for row in self.parser.screen().rows(0, COLS) {
            out.push_str(row.trim_end());
            out.push('\n');
        }
        out
    }
}

/// The one fixture both cases launch, and the scratch home it lives in.
pub(crate) fn fixture(tag: &str) -> PathBuf {
    // /tmp directly: a shorter path than the per-user temp dir, so the 80-col
    // context line keeps the posture segments (the workspace segment yields
    // first by fit; the posture never does).
    let home = PathBuf::from("/tmp").join(format!(
        "saya-resume-posture-{}-{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("the scratch home builds");

    let config = home.join("config-home").join("saya");
    std::fs::create_dir_all(&config).expect("the user config dir builds");
    std::fs::write(
        config.join("config.toml"),
        "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n\n[session_commands]\ndeny = ['curl']\n",
    )
    .expect("the config writes");
    std::fs::write(
        config.join("connections.toml"),
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\n",
            home.join("demo.sqlite3").display()
        ),
    )
    .expect("the connections write");
    // The launch opens the configured profile's connection (the surface composes
    // its session through the line front's opener), so the file must exist
    // before the surface starts — a zero-byte file is a valid empty database.
    std::fs::write(home.join("demo.sqlite3"), b"").expect("the demo db file is created");

    let project = home.join("project");
    std::fs::create_dir_all(&project).expect("the project dir");
    seed_resumed_record(&home, &project.display().to_string());
    home
}

/// Seeds the resumed record: a session whose pin binds the fixture project
/// dir, so a resumed launch recomposes the route exactly as production would
/// (`fresh: false`, root bound → route composes). The user-layer deny list
/// rides the config the child launches with.
fn seed_resumed_record(home: &Path, project: &str) {
    let store = FsSessionStore::new(home.join("sessions"));
    let mut state = SessionState::new(RESUMED_ID, None, "smoke");
    state.workspace_root = Some(project.to_owned());
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test tokio runtime builds")
        .block_on(store.save(state.redacted()))
        .expect("the resumed record saves");
}

/// Launches the real binary over `home`, in a launch folder of its own, under
/// the pty the case reads.
pub(crate) fn launch(home: &Path, args: &[&str]) -> Session {
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: ROWS,
        cols: COLS,
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
    let launch_dir = home.join("launch-dir");
    std::fs::create_dir_all(&launch_dir).expect("the launch dir builds");
    cmd.cwd(launch_dir);
    cmd.args(args);
    let child = pair.slave.spawn_command(cmd).expect("the binary spawns");
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .expect("the pty reader clones");
    Session {
        home: home.to_path_buf(),
        child,
        rx: reader_channel(reader),
        parser: vt100::Parser::new(ROWS, COLS, 0),
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
