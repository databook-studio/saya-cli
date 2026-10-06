//! Verbose-session metrics, printed only after the terminal is restored
//! (packet PH-G1): a real-PTY ordering proof.
//!
//! The test spawns the real `saya --verbose` TUI on a pty, waits for the
//! first paint, answers the startup trust modal, quits with Ctrl+C twice,
//! and reads every raw byte to EOF. The assertions are positional on the
//! raw stream: nothing metric-related may print while the alternate screen
//! is up, and the session metrics section, carrying exactly one first_paint
//! sample line, must land after the alternate-screen leave.
#![cfg(not(windows))]

/// Alternate-screen enter, byte-exact; never typed as an escape sequence.
const ALT_ENTER: [u8; 8] = [0x1b, b'[', b'?', b'1', b'0', b'4', b'9', b'h'];
/// Alternate-screen leave, byte-exact.
const ALT_LEAVE: [u8; 8] = [0x1b, b'[', b'?', b'1', b'0', b'4', b'9', b'l'];
/// The fixed metrics-section header needle.
const HEADER_TEXT: &[u8] = b"saya local metrics";
/// The first_paint sample line the restored section must carry, exactly once.
const FIRST_PAINT_LINE: &[u8] = b"phase first_paint samples=1";
/// How long any single interaction may take to paint.
const INTERACTIVE_WAIT: Duration = Duration::from_secs(15);
/// First (leftmost) position of `needle` in `haystack`.
fn position_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Last (rightmost) position of `needle` in `haystack`.
fn rposition_of(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// How many times `needle` occurs in `haystack`.
fn count_of(haystack: &[u8], needle: &[u8]) -> usize {
    if haystack.len() < needle.len() {
        return 0;
    }
    haystack
        .windows(needle.len())
        .filter(|w| *w == needle)
        .count()
}

/// The readable tail of the raw stream, for failure messages.
fn raw_tail(raw: &[u8]) -> String {
    let start = raw.len().saturating_sub(4000);
    String::from_utf8_lossy(&raw[start..]).into_owned()
}
use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
/// Hard ceiling on the whole test so it can never hang CI. Hitting this is a
/// failure (the screen never settled) and panics with the captured screen.
const HARD_DEADLINE: Duration = Duration::from_secs(30);
/// How long a single blocking read waits before we loop back to check whether
/// the screen has settled. Short enough that the settle check runs even when
/// the TUI has gone quiet (which is the whole point: we wait for it to stop
/// changing), long enough that an idle loop isn't a busy-wait.
const READ_TIMEOUT: Duration = Duration::from_millis(250);

const COLS: u16 = 80;
const ROWS: u16 = 24;
/// The child command every rung and the interactive harness share: a fresh
/// `saya` on the scratch HOME, outside any git worktree (the cwd comment in
/// `paint_with_config` explains why), offline and read-only.
fn tui_command(home: &Path, config: &Path, connections: &Path) -> CommandBuilder {
    let mut cmd = CommandBuilder::new(saya_bin());
    cmd.env("HOME", home);
    // Keep the developer's real config/session/state out of this run.
    cmd.env("SAYA_CONFIG_HOME", home.join("config-home"));
    cmd.env("SAYA_SESSION_DIR", home.join("sessions"));
    cmd.env("SAYA_STATE_DB", home.join("state.sqlite3"));
    // The child must launch outside any git worktree: the harness's own cwd
    // is inside saya's worktree (linked `.git` file), and the child inherits
    // it — so the trust decision would find a bound root and never open its
    // modal. A scratch dir under the test's HOME is outside every worktree,
    // which is exactly the unbound shape the modal covers.
    cmd.cwd(home.join("launch-dir"));
    std::fs::create_dir_all(home.join("launch-dir")).expect("scratch launch dir builds");
    // No XDG/APPDATA fallback can reach the real user dir now.
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    // No provider env can leak in (and we never want to log one).
    cmd.env_remove("SAYA_API_KEY");
    cmd.env_remove("SAYA_AI_API_KEY");
    cmd.args([
        "--config",
        config.to_str().unwrap(),
        "--connections",
        connections.to_str().unwrap(),
        // Read-only approval so nothing could prompt for a query (we send
        // none, but this keeps the startup path fully offline and safe).
        "--approval-mode",
        "read-only",
    ]);
    // An explicit workspace answers the trust decision before it is asked:
    // no startup modal, so the quit below drives the bare interface.
    cmd.args(["--workspace", home.join("launch-dir").to_str().unwrap()]);
    cmd.arg("--verbose");
    cmd
}
/// Pump the pty output onto a channel from a dedicated reader thread so the
/// caller can wait with a timeout (pty `Read` is blocking with no portable
/// deadline). The thread owns the reader and ships `Ok(Vec<u8>)` chunks;
/// `Ok(empty)` signals EOF, `Err` a read failure.
fn reader_channel(
    reader: Box<dyn Read + Send>,
) -> std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>> {
    let (tx, rx) = std::sync::mpsc::channel::<std::io::Result<Vec<u8>>>();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = vec![0u8; 4096];
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
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        }
    });
    rx
}
/// Kills the child process on drop so the test never leaks a running `saya`,
/// even on a timeout panic. `try_wait` first so a clean exit isn't signalled.
struct ChildGuard {
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl ChildGuard {
    fn new(child: Box<dyn portable_pty::Child + Send + Sync>) -> Self {
        Self { child }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.try_wait();
        let _ = self.child.kill();
        let _ = self.child.try_wait();
    }
}

/// Removes the scratch HOME on drop so a passed/failed/panicked test leaves no
/// trace. The directory was created fresh by `scratch_home`, so it is safe to
/// remove wholesale.
struct HomeGuard(PathBuf);

impl Drop for HomeGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
/// Resolves the real `saya` binary cargo builds for integration tests. Never
/// hardcodes a path — `CARGO_BIN_EXE_saya` is provided by cargo at build time.
fn saya_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_saya"))
}

/// A throwaway HOME under the test's temp dir, removed first in case a prior
/// crashed run left it behind. The test cleans it up on exit (see DROP).
///
/// `tag` is a per-test discriminator: cargo runs the tests in one binary in
/// parallel and they share a PID, so `std::process::id()` alone would point
/// every test at the same directory and they would clobber each other's
/// `config.toml`/`connections.toml`. The tag keeps each test's scratch HOME
/// distinct.
fn scratch_home(tag: &str) -> PathBuf {
    let home =
        std::env::temp_dir().join(format!("saya-tui-pty-smoke-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}
/// Pays the first-launch cost once: a freshly linked binary can be slow to
/// start on macOS, and the paint waits below must measure the TUI, not the
/// linker.
fn warm_up_binary() {
    let mut child = std::process::Command::new(saya_bin())
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
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The offline scratch config the verbose TUI child binds: one never-opened
/// sqlite profile, no provider, no memory section.
fn write_offline_config(home: &Path) {
    let config = home.join("config.toml");
    std::fs::write(&config, "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n")
        .expect("interactive config writes");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\nread_only = false\n",
            home.join("demo.sqlite3").display()
        ),
    )
    .expect("interactive connections write");
    // The connector opens sqlite with `create_if_missing(false)`, so the
    // database file must exist before the first query. A zero-byte file is
    // a valid empty sqlite database — `SELECT 1` needs nothing more.
    std::fs::write(home.join("demo.sqlite3"), b"").expect("the demo db file is created");
}

/// Spawns the verbose TUI child on a fresh pty WITHOUT killing it after the
/// paint: the caller quits it normally and the raw stream drains to EOF.
fn spawn_raw_tui(home: &Path) -> Result<RawSession, String> {
    let config = home.join("config.toml");
    let connections = home.join("connections.toml");
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not allocate pty: {e}"))?;
    let child = pair
        .slave
        .spawn_command(tui_command(home, &config, &connections))
        .map_err(|e| format!("could not spawn saya on pty: {e}"))?;
    drop(pair.slave);
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| format!("could not take pty writer: {e}"))?;
    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not clone pty reader: {e}"))?;
    let rx = reader_channel(reader);
    Ok(RawSession {
        writer,
        parser: vt100::Parser::new(ROWS, COLS, 0),
        rx,
        raw: Vec::new(),
        child: ChildGuard::new(child),
    })
}

/// A live TUI child whose every raw byte is accumulated: the ordering proof
/// runs over `raw`, and the parsed screen only steers the waits.
struct RawSession {
    writer: Box<dyn Write + Send>,
    parser: vt100::Parser,
    rx: std::sync::mpsc::Receiver<std::io::Result<Vec<u8>>>,
    raw: Vec<u8>,
    child: ChildGuard,
}
impl RawSession {
    /// The parsed screen as text rows joined by newline characters.
    fn screen(&self) -> String {
        let mut out = String::new();
        for row in self.parser.screen().rows(0, COLS) {
            out.push_str(row.trim_end());
            out.push(char::from(10u8));
        }
        out
    }

    /// Writes raw bytes to the pty; the TUI reads them as key events.
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("pty write");
        self.writer.flush().expect("pty flush");
    }
    /// One bounded read: feeds `raw` and the parser; true exactly at EOF.
    fn pump(&mut self) -> Result<bool, String> {
        match self.rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => Ok(true),
            Ok(Ok(bytes)) => {
                self.raw.extend_from_slice(&bytes);
                self.parser.process(&bytes);
                Ok(false)
            }
            Ok(Err(e)) => Err(format!("pty read failed: {e}")),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Ok(true),
        }
    }
    /// Boundedly waits until the screen shows `needle`.
    fn wait_for(&mut self, needle: &str, within: Duration) {
        let start = Instant::now();
        loop {
            if self.screen().contains(needle) {
                return;
            }
            assert!(
                start.elapsed() < within,
                "never saw {needle:?} in time; screen: {}",
                self.screen()
            );
            match self.pump() {
                Ok(true) => panic!("the TUI exited before {needle:?} appeared"),
                Ok(false) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
    /// Boundedly waits until `needle` has left the screen.
    fn wait_gone(&mut self, needle: &str, within: Duration) {
        let start = Instant::now();
        loop {
            if !self.screen().contains(needle) {
                return;
            }
            assert!(
                start.elapsed() < within,
                "{needle:?} never left; screen: {}",
                self.screen()
            );
            match self.pump() {
                Ok(true) => panic!("the TUI exited while waiting for {needle:?} to leave"),
                Ok(false) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
}
#[test]
fn tui_verbose_session_metrics_print_after_terminal_restoration() {
    warm_up_binary();
    let home = scratch_home("metrics");
    let _cleanup = HomeGuard(home.clone());
    write_offline_config(&home);
    let mut tui = spawn_raw_tui(&home).expect("the verbose TUI spawns on a pty");
    // Wait for the first paint: any non-empty screen text, bounded.
    let start = Instant::now();
    loop {
        if !tui.screen().trim().is_empty() {
            break;
        }
        assert!(start.elapsed() < HARD_DEADLINE, "the TUI never painted");
        match tui.pump() {
            Ok(true) => panic!("the TUI exited before its first paint"),
            Ok(false) => {}
            Err(e) => panic!("{e}"),
        }
    }
    // Quit normally through the interface's own exit command. Typing `/`
    // opens the command menu, so the sequence is type, Esc (close the menu —
    // the menu leaving the screen proves the key was consumed), then Enter
    // (submit): the submitted line reaches the same dispatch every smoke
    // test drives, and the pending tick turns it into a clean exit.
    tui.send(b"/quit");
    tui.wait_for("Exit the REPL", INTERACTIVE_WAIT);
    tui.send(&[0x1b]);
    tui.wait_gone("Exit the REPL", INTERACTIVE_WAIT);
    tui.send(&[13u8]);
    // Drain every remaining byte, bounded. The child quit on its own, so a
    // read error past exit is the pty's end-of-stream (EIO), not a failure;
    // the grace window catches the restoration bytes after the last read.
    let start = Instant::now();
    loop {
        let exited = tui
            .child
            .child
            .try_wait()
            .is_ok_and(|status| status.is_some());
        match tui.pump() {
            Ok(true) => break,
            Ok(false) if exited && start.elapsed() >= Duration::from_millis(500) => break,
            Ok(false) => {}
            Err(_) if exited => break,
            Err(e) => panic!("{e}"),
        }
        assert!(
            start.elapsed() < HARD_DEADLINE,
            "the child never reached EOF"
        );
    }
    // EOF on the pty master arrives when the child closes its terminal, which
    // can be a few milliseconds BEFORE the process is reapable: poll for the
    // exit (bounded) instead of sampling it once at EOF.
    let exit_deadline = Instant::now() + Duration::from_secs(5);
    let exited = loop {
        if tui
            .child
            .child
            .try_wait()
            .is_ok_and(|status| status.is_some())
        {
            break true;
        }
        if Instant::now() >= exit_deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        exited,
        "the TUI child must exit on its own; the harness must never kill it"
    ); // (a) The metrics header lands only after the alternate-screen leave:
    // the terminal was restored before anything metric printed.
    let raw = tui.raw.clone();
    let enter_first = position_of(&raw, &ALT_ENTER).expect("the alternate screen was entered");
    let leave_last = rposition_of(&raw, &ALT_LEAVE).expect("the alternate screen was left");
    assert!(enter_first < leave_last, "the leave must follow the enter");
    let header_first = position_of(&raw, HEADER_TEXT)
        .expect("no metrics header anywhere in the stream: nothing printed after restoration");
    assert!(
        header_first > leave_last,
        "the metrics header must print after the terminal is restored; tail: {}",
        raw_tail(&raw)
    );
    // (b) Nothing metric-related while the TUI owned the screen: neither
    // between the enter and the leave, nor anywhere before the enter.
    let owned = &raw[enter_first..leave_last];
    assert!(
        position_of(owned, HEADER_TEXT).is_none() && position_of(owned, FIRST_PAINT_LINE).is_none(),
        "metric text printed while the TUI owned the screen; tail: {}",
        raw_tail(&raw)
    ); // (c) After the leave: the first_paint sample line, exactly once.
    let after = &raw[leave_last..];
    assert_eq!(
        count_of(after, FIRST_PAINT_LINE),
        1,
        "exactly one first_paint sample line after restoration; tail: {}",
        raw_tail(&raw)
    );
    assert_eq!(
        count_of(&raw, FIRST_PAINT_LINE),
        1,
        "no first_paint sample anywhere but the restored section; tail: {}",
        raw_tail(&raw)
    );
}
