// Unix-only, and the reason is a genuine unknown rather than a shrug.
//
// On Windows every test here fails with `TUI did not settle within 30s` and an
// EMPTY captured screen. A pty is allocated, the child is spawned, and nothing
// ever arrives to parse. Two explanations fit and this suite cannot tell them
// apart:
//
//   1. `vt100` cannot read what ConPTY produces. ConPTY runs its own console
//      host and rewrites the stream, so the bytes are not the VT sequence
//      crossterm emitted — a test-side limitation, and the product is fine.
//   2. The TUI genuinely does not paint under ConPTY — a real defect on a
//      platform we ship binaries for.
//
// Distinguishing them needs someone to run `saya` by hand on a Windows host and
// look at the screen. Until that happens, asserting here would either fail
// forever or be softened until it proved nothing, and *guessing* which
// explanation holds is how the earlier version of this comment came to blame
// ConPTY for what was actually a store-open bug fixed in `saya-store`.
//
// What is lost: the TUI-paints assertion covers macOS and Linux only. See the
// tracking issue for the Windows verification this stands in for.
#![cfg(not(windows))]

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
//!
//! The first test runs memory **off** (the original coverage). The remaining
//! tests extend coverage to the **memory-on** startup path (packet P2b,
//! defect #51): the recorded memory demos run `[memory] mode = "assisted"` and
//! the demo tapes captured the REPL exiting at startup with `Error: local
//! state store is unavailable`. They climb three rungs of increasing cost —
//! empty HOME, a prior store write, a real `connection schema --refresh`
//! against the docker pagila — and report which rung reproduces. Either
//! outcome is a successful packet; a reproducing rung lands `#[ignore]`d with
//! the exact condition so the suite stays green and the reproduction is not
//! lost.

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
    let home = scratch_home("off");
    // Clean up the scratch HOME no matter how this test ends — success, a
    // failed assertion, or a timeout panic. It never touches the developer's
    // real HOME or any real saya state.
    let _cleanup = HomeGuard(home.clone());

    let screen = match paint_screen(&home, Memory::Off) {
        Ok(screen) => screen,
        Err(reason) => {
            // Could not allocate a pty (e.g. a constrained CI runner). Skip
            // cleanly rather than fail — the packet requires this.
            eprintln!("skipping tui_pty_smoke: {reason}");
            return;
        }
    };
    assert_splash_and_status(&screen);
}

/// Memory-on startup path (packet P2b, defect #51). The recorded memory demos
/// run `[memory] mode = "assisted"` and the demo tapes captured the REPL
/// exiting at startup with `Error: local state store is unavailable`, the TUI
/// never painting. This is the cheapest rung — memory on, an empty scratch
/// HOME, nothing else — so it isolates the suspect path (eager store open at
/// startup under memory-on) from any pre-existing store state.
///
/// If this passes, #51 did not reproduce at rung 1: an empty HOME with memory on
/// paints the splash and stays up. That is a coverage win, not a failure — say
/// so explicitly, and do not weaken an assertion to make anything pass. If it
/// fails, the captured screen is the reproduction; land it `#[ignore]`d with
/// the exact condition and return a dependency request (the cause is almost
/// certainly outside saya-cli).
#[test]
fn tui_paints_splash_with_memory_on_empty_home() {
    let home = scratch_home("mem-on-empty");
    let _cleanup = HomeGuard(home.clone());

    let screen = match paint_screen(&home, Memory::Assisted) {
        Ok(screen) => screen,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (memory on): {reason}");
            return;
        }
    };
    assert_splash_and_status(&screen);
}

/// Memory-on after a prior store write that needs no database (rung 2). A
/// first `saya` launch reaches the splash, which opens the state store at
/// startup (`reload_at_refs` → `get_schema` → `pool()` with
/// `create_if_missing`), writing `state.sqlite3` plus its `-wal`/`-shm`
/// sidecars. The second launch — same scratch HOME, so the store already
/// exists with sidecars — is the suspect path: a sibling process's WAL
/// checkpoint or a stale sidecar is exactly the condition packet 50's
/// bounded-open fix was written for, and the demos ran `connection schema
/// --refresh` (a store write) right before the REPL launch.
///
/// The first launch is asserted too: it must paint the splash (the rung-1
/// condition, in-place). Then the second launch is asserted against the same
/// positive/negative set. If either fails, the captured screen is the
/// reproduction.
#[test]
fn tui_paints_splash_with_memory_on_after_a_store_write() {
    let home = scratch_home("mem-on-storewrite");
    // One cleanup for the whole rung: the second launch must see the first
    // launch's store, so we do NOT clean between them.
    let _cleanup = HomeGuard(home.clone());

    let first = match paint_screen(&home, Memory::Assisted) {
        Ok(screen) => screen,
        Err(reason) => {
            eprintln!("skipping tui_pty_smoke (memory on, store write): {reason}");
            return;
        }
    };
    assert_splash_and_status_named(&first, "first launch");

    // The first launch opened the store; prove it before asserting the second
    // launch is meaningfully different. If the store was never created, rung 2
    // collapses into rung 1 and the finding is "store write did not happen".
    let state_db = home.join("state.sqlite3");
    if !state_db.exists() {
        panic!(
            "rung 2 precondition failed: first launch did not create {state_db:?} \
             (reload_at_refs may no longer open the store at startup); rung 2 \
             collapses into rung 1"
        );
    }

    let second = paint_screen(&home, Memory::Assisted)
        .expect("second launch pty allocation failed after the first succeeded");
    assert_splash_and_status_named(&second, "second launch (after store write)");
}

/// Memory-on after a real `connection schema --refresh` against the local
/// docker pagila (rung 3). This is the exact demo sequence — `connection
/// schema <profile> --refresh` then a REPL launch — and the most expensive
/// rung because the refresh needs postgres. It **skips cleanly** (not fails)
/// when the database is unreachable, so the suite stays green on a machine
/// without the `databook-postgres` container.
///
/// The refresh is a real store write against a live database (it populates
/// the schema cache the REPL's `reload_at_refs` then reads at startup). If the
/// defect is in the interaction of a freshly-written store and the REPL's
/// startup open — the demos' exact condition — this is the rung that would
/// surface it. The status bar shows `[docker_postgres]`, not `[demo]`.
#[test]
fn tui_paints_splash_with_memory_on_after_schema_refresh() {
    // Cheapest reachability probe: a TCP connect to the docker postgres port.
    // No docker CLI, no.env.saya, no provider key — just the port the
    // container would listen on. Unreachable ⇒ skip, never fail.
    const PAGILA: &str = "127.0.0.1:5434";
    if !tcp_reachable(PAGILA, Duration::from_secs(1)) {
        eprintln!("skipping rung 3: {PAGILA} unreachable (databook-postgres not running)");
        return;
    }

    let home = scratch_home("mem-on-refresh");
    let _cleanup = HomeGuard(home.clone());
    let config = write_docker_config(&home);

    // Seed the store with a real schema cache write. The refresh needs no
    // model — only the DB — so it runs with the offline smoke config. If it
    // fails (container up but pagila missing, wrong password, …) skip rather
    // than assert, per the packet.
    let refresh = run_saya_schema_refresh(&home, &config);
    if !refresh.success {
        eprintln!(
            "skipping rung 3: schema refresh failed (exit {:?}); stderr:\n{}",
            refresh.exit_code, refresh.stderr
        );
        return;
    }

    let screen = match paint_with_config(&home, &config.config, &config.connections) {
        Ok(screen) => screen,
        Err(reason) => {
            eprintln!("skipping rung 3 (pty): {reason}");
            return;
        }
    };
    assert_splash_for_profile(&screen, "docker_postgres", "rung 3 (after schema refresh)");
}

/// Spawns `saya` on a pty, drains its output until the screen settles, and
/// returns the parsed vt100 screen. Returns `Err(String)` only if a pty
/// cannot be allocated or the child cannot be spawned; any other failure
/// (including a timeout) panics with the captured screen text so the failure
/// is diagnosable rather than a silent hang. The child is always killed before
/// returning so the test never leaks a running `saya` process.
fn paint_screen(home: &Path, memory: Memory) -> Result<vt100::Screen, String> {
    let config = write_scratch_config(home, memory);
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
        // An EMPTY screen is not a settled screen. The stability clock alone
        // treats "nothing has been written yet" as stable, so a slow start — a
        // loaded machine, a cold page cache — used to settle on a blank screen
        // and fail the assertion with an empty capture. That is what made this
        // test look like a Windows/ConPTY limitation when it was really a race
        // present on every platform. Require content first; a process that
        // never paints is caught by HARD_DEADLINE instead, which reports the
        // same empty capture but says truthfully that it never settled.
        if !screen_text(parser.screen()).trim().is_empty() && last_change.elapsed() >= SETTLE {
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

struct ScratchConfig {
    config: PathBuf,
    connections: PathBuf,
}

/// Whether the scratch config turns `[memory]` on. `Off` writes no `[memory]`
/// table (the original smoke test); `Assisted` writes `[memory] mode =
/// 'assisted'` — the section every failing demo tape ran.
#[derive(Copy, Clone)]
enum Memory {
    Off,
    Assisted,
}

/// Writes a minimal, offline config + connections file under `home`. No
/// `--env-file`, no provider key, and a single `demo` profile pointing at a
/// scratch SQLite file that is never opened — the test sends no question. The
/// profile name shows up as `[demo]` in the status bar. When `memory` is
/// `Assisted` the config adds `[memory] mode = 'assisted'`, the suspect path
/// for defect #51; `Off` keeps the original no-`[memory]` shape.
fn write_scratch_config(home: &Path, memory: Memory) -> ScratchConfig {
    let config = home.join("config.toml");
    let memory_section = match memory {
        Memory::Off => String::new(),
        Memory::Assisted => "\n[memory]\nmode = 'assisted'\n".to_string(),
    };
    std::fs::write(
        &config,
        // [ai] model only; no provider key. Loads offline. The optional
        // [memory] section is the only difference between the rungs.
        format!("[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n{memory_section}"),
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

/// The docker pagila profile, mirrored from `.saya/connections.toml` so rung 3
/// does not depend on the developer's checked-in file (the test owns its
/// scratch HOME). The password is an env reference; `run_saya_schema_refresh`
/// sets the env value, never inlining it.
const DOCKER_POSTGRES_PASSWORD_ENV: &str = "SAYA_DOCKER_POSTGRES_PASSWORD";

/// Writes a config + connections pointing at the local docker pagila profile
/// (`docker_postgres`, port 5434) with memory on. The model is the offline
/// `smoke` placeholder — the splash and the schema refresh never call the
/// provider — and the password stays an env reference, never inlined. Used by
/// rung 3 only.
fn write_docker_config(home: &Path) -> ScratchConfig {
    let config = home.join("config.toml");
    std::fs::write(
        &config,
        // Memory on, default profile = docker_postgres. No provider key: the
        // splash is offline and `connection schema --refresh` reads no model.
        "[ai]\nmodel = 'smoke'\n\n[run]\nmax_rows = 10\n\n\
         default_profile = 'docker_postgres'\n\n[memory]\nmode = 'assisted'\n",
    )
    .unwrap();

    let connections = home.join("connections.toml");
    std::fs::write(
        &connections,
        // Mirrors.saya/connections.toml's docker_postgres profile. The
        // password is an env reference to DOCKER_POSTGRES_PASSWORD_ENV (the
        // env var name, not the Rust const) — run_saya_schema_refresh sets
        // that env var in the child; the value is never inlined in config.
        format!(
            "[profiles.docker_postgres]\n\
             type = 'postgresql'\n\
             host = '127.0.0.1'\n\
             port = 5434\n\
             database = 'pagila'\n\
             user = 'databook'\n\
             password = {{ env = '{DOCKER_POSTGRES_PASSWORD_ENV}' }}\n\
             sslmode = 'disable'\n"
        ),
    )
    .unwrap();

    ScratchConfig {
        config,
        connections,
    }
}

/// Outcome of a headless `saya connection schema --refresh` run. `success` is
/// the only field the caller branches on; `exit_code` and `stderr` feed the
/// skip message so a clean skip says *why* the database was unusable.
struct SchemaRefreshResult {
    success: bool,
    exit_code: Option<i32>,
    stderr: String,
}

/// Runs `saya connection schema docker_postgres --refresh` headlessly against
/// the scratch HOME, seeding the store's schema cache with a real write
/// against the live pagila database. Sets the docker postgres password in the
/// child env (never inlined in config); never reads or logs it back.
fn run_saya_schema_refresh(home: &Path, config: &ScratchConfig) -> SchemaRefreshResult {
    let output = std::process::Command::new(saya_bin())
        .env("HOME", home)
        .env("SAYA_CONFIG_HOME", home.join("config-home"))
        .env("SAYA_SESSION_DIR", home.join("sessions"))
        .env("SAYA_STATE_DB", home.join("state.sqlite3"))
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("APPDATA")
        .env_remove("SAYA_API_KEY")
        .env_remove("SAYA_AI_API_KEY")
        .env(DOCKER_POSTGRES_PASSWORD_ENV, "databook")
        .args([
            "--config",
            config.config.to_str().unwrap(),
            "--connections",
            config.connections.to_str().unwrap(),
            "--approval-mode",
            "read-only",
            "connection",
            "schema",
            "docker_postgres",
            "--refresh",
        ])
        .output()
        .expect("could not spawn saya for schema refresh");

    SchemaRefreshResult {
        success: output.status.success(),
        exit_code: output.status.code(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// Cheap TCP reachability probe with a short deadline. Used only to decide
/// whether rung 3 should run at all — never as an assertion. `127.0.0.1:5434`
/// is the docker pagila port; a refused/timeout connect means the container is
/// not up and the rung skips cleanly.
fn tcp_reachable(addr: &str, timeout: Duration) -> bool {
    use std::net::TcpStream;
    use std::str::FromStr;
    let socket = match std::net::SocketAddr::from_str(addr) {
        Ok(socket) => socket,
        Err(_) => return false,
    };
    TcpStream::connect_timeout(&socket, timeout).is_ok()
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

/// The positive/negative assertion set every rung shares: the splash must
/// paint and the active profile must appear in the status bar, while none of
/// the fatal-failure strings may appear anywhere on screen. The negatives are
/// the point — a test that only checks the happy string passes on a screen
/// that also contains `Error: local state store is unavailable`, which is
/// exactly the defect (#51) this packet hunts.
fn assert_splash_and_status(screen: &vt100::Screen) {
    assert_splash_for_profile(screen, "demo", "screen");
}

/// Same as [`assert_splash_and_status`] but tags failure messages with `label`
/// so a rung that asserts two launches (or names a non-`demo` profile) points
/// at the one that broke.
fn assert_splash_and_status_named(screen: &vt100::Screen, label: &str) {
    assert_splash_for_profile(screen, "demo", label);
}

/// Like [`assert_splash_and_status`] but checks a profile name other than
/// `demo` — used by rung 3, which points at the docker pagila profile.
fn assert_splash_for_profile(screen: &vt100::Screen, profile: &str, label: &str) {
    let text = screen_text(screen);

    // Positive: the splash the REPL paints on startup must be present.
    assert_in(
        &text,
        "◆ saya",
        &format!("splash marker (◆ saya) missing from {label}"),
        &text,
    );
    assert_in(
        &text,
        "Ask your databases in plain language.",
        &format!("splash tagline missing from {label}"),
        &text,
    );

    // Positive: the status bar names the active profile.
    assert_in(
        &text,
        &format!("[{profile}]"),
        &format!("status bar profile [{profile}] missing from {label}"),
        &text,
    );

    // Negative: none of the fatal-failure strings may appear anywhere on
    // screen. `local state store is unavailable` is the #51 signature; the
    // other two guard against a shell takeover or any other fatal line.
    assert_not_in(
        &text,
        "local state store is unavailable",
        &format!("store-unavailable error painted on {label}"),
        &text,
    );
    assert_not_in(
        &text,
        "command not found",
        &format!("shell error on {label}"),
        &text,
    );
    assert_not_in(
        &text,
        "Error:",
        &format!("fatal Error: line on {label}"),
        &text,
    );
}
