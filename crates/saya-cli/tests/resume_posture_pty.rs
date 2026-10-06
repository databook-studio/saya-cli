// Unix-only: the pty harness behind `tui_pty_smoke.rs` is macOS/Linux, and
// the same ConPTY unknown applies here.
#![cfg(not(windows))]

//! A `--resume` LAUNCH derives the host posture header (REP2b): the real
//! binary, on a pty, with a recorded session whose pin binds a root. The
//! piped REPL never prints the status line (it is TTY-gated), so the pty TUI
//! is the observable header surface: the context line must name the lane the
//! resumed universe composed ("Host commands unsandboxed") and the deny list
//! the user-layer config composed into it ("Denied: curl") — facts that ride
//! `SessionState` and were, before this packet, only derived on a fresh
//! start.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use saya_cli::SessionState;
use saya_store::{FsSessionStore, SessionStore};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const RESUMED_ID: &str = "20260101-000000-pty0";
const COLS: u16 = 80;
const ROWS: u16 = 24;
const READ_TIMEOUT: Duration = Duration::from_millis(250);
const DEADLINE: Duration = Duration::from_secs(30);

struct HomeGuard(PathBuf);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct ChildGuard(Box<dyn portable_pty::Child + Send + Sync>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.try_wait();
    }
}

fn scratch_home(tag: &str) -> PathBuf {
    // /tmp directly: a shorter path than the per-user temp dir, so the 80-col
    // context line keeps the posture segments (the workspace segment yields
    // first by fit; the posture never does).
    let home = PathBuf::from("/tmp").join(format!(
        "saya-resume-posture-{}-{}",
        std::process::id(),
        tag
    ));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

/// Seeds the resumed record: a session whose pin binds the fixture project
/// dir, so a resumed launch recomposes the lane exactly as production would
/// (`fresh: false`, root bound → lane composes). The user-layer deny list
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

fn wait_for(
    rx: &std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    parser: &mut vt100::Parser,
    needle: &str,
) {
    let start = Instant::now();
    loop {
        if screen_text(parser).contains(needle) {
            return;
        }
        assert!(
            start.elapsed() < DEADLINE,
            "never saw {needle:?}; screen:\n{}",
            screen_text(parser)
        );
        match rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => panic!(
                "the child exited before {needle:?} appeared; screen:\n{}",
                screen_text(parser)
            ),
            Ok(Ok(bytes)) => parser.process(&bytes),
            Ok(Err(error)) => panic!("pty read failed: {error}"),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => panic!("pty reader ended"),
        }
    }
}

fn screen_text(parser: &vt100::Parser) -> String {
    let mut out = String::new();
    for row in parser.screen().rows(0, COLS) {
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

#[test]
fn a_resumed_launch_paints_the_host_posture_the_universe_composed() {
    let home = scratch_home("resume");
    let _cleanup = HomeGuard(home.clone());

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

    let project = home.join("project");
    std::fs::create_dir_all(&project).expect("the project dir");
    seed_resumed_record(&home, &project.display().to_string());

    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize {
        rows: ROWS,
        cols: COLS,
        pixel_width: 0,
        pixel_height: 0,
    }) {
        Ok(pair) => pair,
        Err(reason) => {
            eprintln!("skipping resume_posture_pty: could not allocate pty: {reason}");
            return;
        }
    };
    let mut cmd = CommandBuilder::new(PathBuf::from(env!("CARGO_BIN_EXE_saya")));
    cmd.env("HOME", &home);
    cmd.env("SAYA_CONFIG_HOME", home.join("config-home"));
    cmd.env("SAYA_SESSION_DIR", home.join("sessions"));
    cmd.env("SAYA_STATE_DB", home.join("state.sqlite3"));
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    cmd.env_remove("SAYA_API_KEY");
    cmd.env_remove("SAYA_AI_API_KEY");
    cmd.cwd(home.join("launch-dir"));
    std::fs::create_dir_all(home.join("launch-dir")).expect("the launch dir builds");
    cmd.args(["--approval-mode", "read-only", "--resume", RESUMED_ID]);
    let child = match pair.slave.spawn_command(cmd) {
        Ok(child) => child,
        Err(reason) => {
            eprintln!("skipping resume_posture_pty: could not spawn: {reason}");
            return;
        }
    };
    drop(pair.slave);
    let _child = ChildGuard(child);
    let reader = pair
        .master
        .try_clone_reader()
        .expect("the pty reader clones");
    let rx = reader_channel(reader);
    let mut parser = vt100::Parser::new(ROWS, COLS, 0);

    // The context line's posture facts, from the state the launch derived:
    // the resumed universe composed the lane (the pin re-binds), and the
    // config deny list composed into it. The workspace segment yields to fit
    // on a narrow row; the posture segments never do.
    wait_for(&rx, &mut parser, "Host commands unsandboxed");
    wait_for(&rx, &mut parser, "Denied: curl");
}

fn reader_channel(
    reader: Box<dyn Read + Send>,
) -> std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>> {
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
