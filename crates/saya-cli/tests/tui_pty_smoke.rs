//! Smoke test that the full-screen TUI actually paints, from a real process.
//!
//! Spawns the REAL `saya` binary (resolved via `CARGO_BIN_EXE_saya`, the path
//! cargo injects into integration tests — never hardcoded) on a
//! pseudo-terminal, feeds it a throwaway `HOME` + generated config, waits for
//! the screen to settle, then parses the terminal byte stream into a screen
//! and asserts.
//!
//! This is the missing assertion the packet describes. A green exit code and a
//! plausible file size tell you nothing — the REPL can exit at startup with
//! `Error: local state store is unavailable`, the TUI never paints, and every
//! subsequent keystroke goes to the shell. So we assert the splash text IS on
//! screen AND that none of the fatal-failure strings appear anywhere on it.
//!
//! Dev-only dependencies (`portable-pty` for the pty, `vt100` to parse the byte
//! stream into a screen); nothing here is a runtime dependency. No model and no
//! database: the REPL reaches its splash without a provider call, so we send no
//! question and configure a profile that is never opened.

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// How long the parsed screen must be content-stable before we consider it
/// settled and read it. The ratatui TUI redraws on a 60 ms poll cadence even
/// with no input, so we settle on the screen *text* not changing for this
/// window — see `paint_with_config`.
const SETTLE: Duration = Duration::from_secs(3);
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

#[test]
fn tui_paints_splash_and_status_bar_on_a_real_pty() {
    let home = scratch_home();
    // Clean up the scratch HOME no matter how this test ends — success, a
    // failed assertion, or a timeout panic. It never touches the developer's
    // real HOME or any real saya state.
    let _cleanup = HomeGuard(home.clone());

    let screen = match paint_screen(&home) {
        Ok(screen) => screen,
        Err(reason) => {
            // Could not allocate a pty (e.g. a constrained CI runner). Skip
            // cleanly rather than fail — the packet requires this.
            eprintln!("skipping tui_pty_smoke: {reason}");
            return;
        }
    };
    let text = screen_text(&screen);

    // Positive: the splash the REPL paints on startup must be present.
    assert_in(
        &text,
        "◆ saya",
        "splash marker (◆ saya) missing from screen",
        &text,
    );
    assert_in(
        &text,
        "Ask your databases in plain language.",
        "splash tagline missing from screen",
        &text,
    );

    // Positive: the status bar names the active profile as `[demo]`.
    assert_in(&text, "[demo]", "status bar profile [demo] missing", &text);

    // Negative: none of the fatal-failure strings may appear anywhere on
    // screen. This is the point — a test that only checks the happy string
    // passes on a screen that also contains a fatal error.
    assert_not_in(
        &text,
        "local state store is unavailable",
        "store-unavailable error painted on screen",
        &text,
    );
    assert_not_in(&text, "command not found", "shell error on screen", &text);
    assert_not_in(&text, "Error:", "fatal Error: line on screen", &text);
}

/// Spawns `saya` on a pty, drains its output until the screen settles, and
/// returns the parsed vt100 screen. Returns `Err(String)` only if a pty
/// cannot be allocated or the child cannot be spawned; any other failure
/// (including a timeout) panics with the captured screen text so the failure
/// is diagnosable rather than a silent hang. The child is always killed before
/// returning so the test never leaks a running `saya` process.
fn paint_screen(home: &Path) -> Result<vt100::Screen, String> {
    let config = write_scratch_config(home);
    paint_with_config(home, &config.config, &config.connections)
}

fn paint_with_config(
    home: &Path,
    config: &Path,
    connections: &Path,
) -> Result<vt100::Screen, String> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: ROWS,
            cols: COLS,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| format!("could not allocate pty: {e}"))?;

    let mut cmd = CommandBuilder::new(saya_bin());
    cmd.env("HOME", home);
    // Keep the developer's real config/session/state out of this run.
    cmd.env("SAYA_CONFIG_HOME", home.join("config-home"));
    cmd.env("SAYA_SESSION_DIR", home.join("sessions"));
    cmd.env("SAYA_STATE_DB", home.join("state.sqlite3"));
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

    // Spawn the child on the pty's slave side. `spawn_command` lives on
    // `SlavePty`, not `PtySystem`, in portable-pty 0.9 — so we must spawn
    // before dropping the slave. The child inherits the slave end; keeping
    // `child` alive holds the pty open while we read from the master.
    let child = pair
        .slave
        .spawn_command(cmd)
        .map_err(|e| format!("could not spawn saya on pty: {e}"))?;
    // We've spawned; the slave handle is no longer needed.
    drop(pair.slave);

    // Always kill the child when we leave this function, whether we settled,
    // hit EOF, or panicked. The TUI event loop otherwise waits for input
    // forever.
    let _child = ChildGuard::new(child);

    let reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| format!("could not clone pty reader: {e}"))?;

    // Pump the pty output onto a channel from a dedicated reader thread so the
    // main loop can wait with a timeout (pty `Read` is blocking with no
    // portable deadline). The thread owns the reader and ships `Ok(Vec<u8>)`
    // chunks; `Ok(empty)` signals EOF, `Err` a read failure.
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

    let mut parser = vt100::Parser::new(ROWS, COLS, 0);
    let start = Instant::now();
    // Settle by CONTENT stability, not byte-quiet: the ratatui TUI redraws on
    // every 60 ms poll even with no input, so bytes keep flowing after the
    // screen is visually done. vt100 is idempotent — re-processing a redraw
    // yields the same screen text — so we settle when the parsed screen stops
    // CHANGING for SETTLE, not when the stream goes quiet.
    let mut prev = String::new();
    let mut last_change = Instant::now();

    loop {
        let elapsed = start.elapsed();
        if elapsed >= HARD_DEADLINE {
            panic!(
                "TUI did not settle within {HARD_DEADLINE:?}; captured screen:\n{}",
                screen_text(parser.screen())
            );
        }
        if last_change.elapsed() >= SETTLE {
            // The screen content has been stable long enough; return it.
            return Ok(parser.screen().clone());
        }

        match rx.recv_timeout(READ_TIMEOUT) {
            Ok(Ok(bytes)) if bytes.is_empty() => {
                // EOF before settle: the process exited. This is itself a
                // finding — assert against whatever painted before returning.
                return Ok(parser.screen().clone());
            }
            Ok(Ok(bytes)) => {
                parser.process(&bytes);
                let now = screen_text(parser.screen());
                if now != prev {
                    prev = now;
                    last_change = Instant::now();
                }
            }
            Ok(Err(e)) => panic!(
                "pty read failed: {e}; screen:\n{}",
                screen_text(parser.screen())
            ),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                // Reader thread ended without an explicit EOF message
                // (receiver dropped). Treat as EOF.
                return Ok(parser.screen().clone());
            }
        }
    }
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

/// Renders the vt100 screen as plain text rows joined by newlines, trailing
/// whitespace trimmed per row so the assertions don't depend on column
/// padding. `Screen::rows(0, cols)` yields one `String` per row without
/// newlines.
fn screen_text(screen: &vt100::Screen) -> String {
    let mut out = String::new();
    for row in screen.rows(0, COLS) {
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// Resolves the real `saya` binary cargo builds for integration tests. Never
/// hardcodes a path — `CARGO_BIN_EXE_saya` is provided by cargo at build time.
fn saya_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_saya"))
}

/// A throwaway HOME under the test's temp dir, removed first in case a prior
/// crashed run left it behind. The test cleans it up on exit (see DROP).
fn scratch_home() -> PathBuf {
    let home = std::env::temp_dir().join(format!("saya-tui-pty-smoke-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home
}

struct ScratchConfig {
    config: PathBuf,
    connections: PathBuf,
}

/// Writes a minimal, offline config + connections file under `home`. Memory
/// is off (no `[memory]` table, no provider call at startup), no `--env-file`,
/// and a single `demo` profile pointing at a scratch SQLite file that is never
/// opened — the test sends no question. The profile name shows up as `[demo]`
/// in the status bar.
fn write_scratch_config(home: &Path) -> ScratchConfig {
    let config = home.join("config.toml");
    std::fs::write(
        &config,
        // [ai] model only; no provider key, no memory. Loads offline.
        "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n",
    )
    .unwrap();

    let database = home.join("demo.sqlite3");
    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        format!(
            "[profiles.demo]\ntype = 'sqlite'\npath = '{}'\n",
            database.display()
        ),
    )
    .unwrap();

    ScratchConfig {
        config,
        connections,
    }
}

// ----- assertions that print the captured screen on failure -----------------

fn assert_in(haystack: &str, needle: &str, msg: &str, screen: &str) {
    assert!(
        haystack.contains(needle),
        "{msg}\n--- captured screen ---\n{screen}"
    );
}

fn assert_not_in(haystack: &str, needle: &str, msg: &str, screen: &str) {
    assert!(
        !haystack.contains(needle),
        "{msg}: {needle:?} appeared on screen\n--- captured screen ---\n{screen}"
    );
}
