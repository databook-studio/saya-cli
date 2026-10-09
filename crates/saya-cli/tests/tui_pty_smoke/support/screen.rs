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

    let cmd = tui_command(home, config, connections);

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
    let rx = reader_channel(reader);

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
