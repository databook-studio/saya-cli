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
