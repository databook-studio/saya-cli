//! O04 `/reload` on the real binary: the piped REPL, `--turn-file`, and the
//! privacy regression. Piped stdin/stdout keeps this the headless path (no
//! trust prompt, no editor), so the loop reads lines and prints results we
//! can wait for — a reader thread per stream plus a wait-for-substring
//! helper, as `tests/mvp/session.rs` does. The config home is isolated via
//! `SAYA_CONFIG_HOME`, and every inherited `SAYA_*` variable is removed so
//! the reload re-reads exactly the files this test writes.
//!
//! No provider and no database: slash commands need neither.

use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

const DEADLINE: Duration = Duration::from_secs(10);

struct ChildGuard(Option<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            if child.try_wait().unwrap().is_none() {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

struct Streams {
    stdout: Arc<Mutex<Vec<u8>>>,
    stderr: Arc<Mutex<Vec<u8>>>,
}

fn drain(buffer: Arc<Mutex<Vec<u8>>>, mut reader: Box<dyn Read + Send>) {
    let mut chunk = [0_u8; 4096];
    while let Ok(size) = reader.read(&mut chunk) {
        if size == 0 {
            break;
        }
        buffer.lock().unwrap().extend_from_slice(&chunk[..size]);
    }
}

fn wait_for(streams: &Streams, needle: &str, label: &str) {
    let deadline = Instant::now() + DEADLINE;
    loop {
        {
            let stdout_guard = streams.stdout.lock().unwrap();
            let stdout = String::from_utf8_lossy(&stdout_guard);
            if stdout.contains(needle) {
                return;
            }
            let stderr_guard = streams.stderr.lock().unwrap();
            let stderr = String::from_utf8_lossy(&stderr_guard);
            if stderr.contains(needle) {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "did not observe {label} ({needle})"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn scratch_root(label: &str) -> std::path::PathBuf {
    let root = std::env::temp_dir().join(format!(
        "saya-reload-repl-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

/// The user config file the reload re-reads: `SAYA_CONFIG_HOME/saya` is the
/// user config dir, so its `config.toml` is the selected config when no
/// `--config` flag is passed.
fn user_config(root: &std::path::Path) -> std::path::PathBuf {
    root.join("cfg").join("saya").join("config.toml")
}

/// The isolated launch environment: every inherited `SAYA_*` variable is
/// removed, then the ones the test controls are set, so the reload re-reads
/// exactly the files this test writes.
fn isolated(command: &mut Command, root: &std::path::Path) {
    for (key, _) in std::env::vars_os() {
        let key = key.to_string_lossy().into_owned();
        if key.starts_with("SAYA_") {
            command.env_remove(&key);
        }
    }
    command
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("APPDATA")
        .env("SAYA_CONFIG_HOME", root.join("cfg"))
        .env("SAYA_SESSION_DIR", root.join("sessions"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"));
    std::fs::create_dir_all(root.join("sessions")).unwrap();
}

fn spawn_piped(root: &std::path::Path) -> (ChildGuard, std::process::ChildStdin, Streams) {
    let workspace = root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .arg("--workspace")
        .arg(&workspace)
        .arg("--approval-mode")
        .arg("read-only")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    isolated(&mut command, root);
    let mut child = command.spawn().unwrap();
    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();
    let streams = Streams {
        stdout: Arc::new(Mutex::new(Vec::new())),
        stderr: Arc::new(Mutex::new(Vec::new())),
    };
    thread::spawn({
        let sink = Arc::clone(&streams.stdout);
        move || drain(sink, Box::new(stdout))
    });
    thread::spawn({
        let sink = Arc::clone(&streams.stderr);
        move || drain(sink, Box::new(stderr))
    });
    let stdin = child.stdin.take().unwrap();
    (ChildGuard(Some(child)), stdin, streams)
}

fn say(stdin: &mut std::process::ChildStdin, line: &str) {
    use std::io::Write;
    stdin.write_all(format!("{line}\n").as_bytes()).unwrap();
    stdin.flush().unwrap();
}

/// The full piped-REPL arc: unchanged, edited, broken, restored, exit 0.
#[test]
fn reload_piped_repl_applies_edits_and_survives_a_broken_file() {
    let root = scratch_root("arc");
    std::fs::create_dir_all(user_config(&root).parent().unwrap()).unwrap();
    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();

    let (mut guard, mut stdin, streams) = spawn_piped(&root);

    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "Configuration is unchanged.",
        "the unchanged reload",
    );

    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 5\n",
    )
    .unwrap();
    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "max_rows: 100 -> 5",
        "the edited reload names the change",
    );

    std::fs::write(user_config(&root), "[run\n").unwrap();
    say(&mut stdin, "/reload");
    wait_for(&streams, "Reload failed: ", "the broken file fails");
    wait_for(
        &streams,
        "The previous configuration is still in use.",
        "the pinned sentence",
    );

    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 7\n",
    )
    .unwrap();
    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "max_rows: 5 -> 7",
        "the restored reload works again",
    );

    say(&mut stdin, "/exit");
    let mut child = guard.0.take().unwrap();
    let deadline = Instant::now() + DEADLINE;
    let code = loop {
        if let Some(code) = child.try_wait().unwrap() {
            break code.code().unwrap_or(-1);
        }
        assert!(Instant::now() < deadline, "the child never exited");
        thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(code, 0, "the session ends cleanly after reloads");
    let _ = std::fs::remove_dir_all(root);
}

/// The privacy regression at the real surface: a config that widens
/// `allow_data_sharing` is reloaded but NOT applied — the plan lists it
/// under `Not applied in this session:` and a following `/privacy` still
/// reports the session's unchanged value (`off`).
#[test]
fn reload_piped_repl_never_widens_sharing_and_privacy_says_so() {
    let root = scratch_root("privacy");
    std::fs::create_dir_all(user_config(&root).parent().unwrap()).unwrap();
    std::fs::write(user_config(&root), "[ai]\nallow_data_sharing = false\n").unwrap();

    let (_guard, mut stdin, streams) = spawn_piped(&root);

    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "Configuration is unchanged.",
        "the unchanged reload",
    );

    std::fs::write(user_config(&root), "[ai]\nallow_data_sharing = true\n").unwrap();
    say(&mut stdin, "/reload");
    wait_for(
        &streams,
        "Not applied in this session:",
        "the widen is classified pending",
    );
    {
        let guard = streams.stdout.lock().unwrap();
        let stdout = String::from_utf8_lossy(&guard);
        assert!(
            stdout.contains("allow_data_sharing: false -> true"),
            "the plan names the widen: {stdout}"
        );
    }

    say(&mut stdin, "/privacy");
    wait_for(
        &streams,
        "Cloud data sharing: off",
        "/privacy still says off",
    );
    let _ = std::fs::remove_dir_all(root);
}

/// `--turn-file /reload`: one verbatim turn, the plan printed, exit 0.
#[test]
fn turn_file_reload_prints_the_plan_and_exits_zero() {
    let root = scratch_root("turn-file");
    std::fs::create_dir_all(user_config(&root).parent().unwrap()).unwrap();
    std::fs::write(
        user_config(&root),
        "[ai]\nmodel = \"m1\"\n[run]\nmax_rows = 100\n",
    )
    .unwrap();
    let turn = root.join("turn.txt");
    std::fs::write(&turn, "/reload").unwrap();

    let workspace = root.join("ws");
    std::fs::create_dir_all(&workspace).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_saya"));
    command
        .arg("--workspace")
        .arg(&workspace)
        .arg("--approval-mode")
        .arg("read-only")
        .arg("--turn-file")
        .arg(&turn);
    isolated(&mut command, &root);
    let output = command.output().unwrap();

    assert_eq!(output.status.code(), Some(0), "exit 0: {:?}", output.status);
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Configuration is unchanged."),
        "the plan is printed: {stdout}"
    );
    let _ = std::fs::remove_dir_all(root);
}
