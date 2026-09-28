//! Binary tests for `saya setup` (S16): the non-interactive refusal (exit 2,
//! stdin untouched), the interrupted-setup startup warning on every other
//! command, completions staying silent, and one full guided flow through a
//! real pty. Every run is isolated to a scratch config home; nothing touches
//! the developer's files.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

fn isolated_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("saya-cli-setup-{label}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    root
}

/// The binary pointed at the scratch root, launched from it too, so no
/// project layer and no real user config can interfere.
fn saya_command(root: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_saya"));
    cmd.current_dir(root)
        .env("SAYA_CONFIG_HOME", root.join("config-home"))
        .env("SAYA_STATE_DB", root.join("state.sqlite3"))
        .env("HOME", root)
        .env_remove("XDG_CONFIG_HOME")
        .env_remove("XDG_DATA_HOME")
        .env_remove("APPDATA");
    cmd
}

fn stderr_of(output: &std::process::Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Writes a valid pending-commit marker (one appended connections.toml entry)
/// into the scratch user config directory.
fn stage_marker(root: &Path) -> PathBuf {
    let user_dir = root.join("config-home").join("saya");
    fs::create_dir_all(&user_dir).unwrap();
    let marker = serde_json::json!({
        "version": 1,
        "started_unix_ms": 42,
        "entries": [
            { "file": "connections.toml", "backup": "connections.toml", "created": false }
        ]
    });
    fs::write(
        user_dir.join(".setup-commit.json"),
        serde_json::to_vec(&marker).unwrap(),
    )
    .unwrap();
    user_dir
}

#[test]
fn setup_noninteractive_never_prompts() {
    let root = isolated_root("noninteractive");
    // Both spellings refuse: the explicit flag, and stdin that cannot be a
    // terminal (/dev/null). Neither may read stdin or write a file — a flow
    // that prompted would have hit EOF and cancelled with exit 0 instead.
    for args in [vec!["setup", "--non-interactive"], vec!["setup"]] {
        let stdin = fs::File::open("/dev/null").unwrap();
        let out = saya_command(&root)
            .args(&args)
            .stdin(Stdio::from(stdin))
            .output()
            .unwrap();
        let stderr = stderr_of(&out);
        assert_eq!(out.status.code(), Some(2), "exit 2 for {args:?}: {stderr}");
        assert!(
            stderr.contains("saya setup is interactive"),
            "the guidance names the interactive requirement: {stderr}"
        );
        assert!(
            stderr.contains("saya config init") && stderr.contains("saya demo"),
            "the guidance offers the scripted alternatives: {stderr}"
        );
        assert!(
            !root.join("config-home/saya/connections.toml").exists()
                && !root.join("config-home/saya/config.toml").exists(),
            "no files were written for {args:?}"
        );
    }
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn startup_warning_printed_when_a_marker_exists() {
    let root = isolated_root("warning");
    let user_dir = stage_marker(&root);
    let out = saya_command(&root)
        .args(["--non-interactive", "config", "show"])
        .output()
        .unwrap();
    let stderr = stderr_of(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "the command still succeeds: {stderr}"
    );
    assert!(
        stderr.contains("An interrupted `saya setup` left a recovery marker"),
        "the warning names the interruption: {stderr}"
    );
    assert!(
        stderr.contains(user_dir.to_str().unwrap()),
        "the warning names the directory: {stderr}"
    );
    assert!(
        user_dir.join(".setup-commit.json").exists(),
        "never auto-restores"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn startup_marker_read_error_is_a_warning_not_a_crash() {
    let root = isolated_root("warning-corrupt");
    let user_dir = root.join("config-home").join("saya");
    fs::create_dir_all(&user_dir).unwrap();
    fs::write(user_dir.join(".setup-commit.json"), b"not json").unwrap();
    let out = saya_command(&root)
        .args(["--non-interactive", "config", "show"])
        .output()
        .unwrap();
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "no crash: {stderr}");
    assert!(
        stderr.contains("could not check for an interrupted"),
        "the read error is a single warning: {stderr}"
    );
    assert!(
        !stderr.contains("recovery marker in"),
        "no false pending claim: {stderr}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn completions_stay_silent_when_a_marker_exists() {
    let root = isolated_root("warning-completions");
    stage_marker(&root);
    let out = saya_command(&root)
        .args(["completions", "--shell", "bash"])
        .output()
        .unwrap();
    let stderr = stderr_of(&out);
    assert_eq!(out.status.code(), Some(0), "{stderr}");
    assert!(
        !stderr.contains("recovery marker"),
        "completions never carry the setup warning: {stderr}"
    );
    let _ = fs::remove_dir_all(&root);
}

/// The full guided flow through a real pty: skip the provider, add a sqlite
/// profile, watch the probe pass, confirm — files written, exit 0, no marker.
/// Unix-only: pty allocation; the headless paths above cover the rest.
#[test]
#[cfg(not(windows))]
fn setup_full_sqlite_flow_through_a_pty() {
    use portable_pty::{CommandBuilder, PtySize, native_pty_system};

    let root = isolated_root("pty");
    // An empty file is a valid empty SQLite database, so the probe succeeds.
    let db_file = root.join("team.db");
    fs::File::create(&db_file).unwrap();

    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 40,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .expect("pty allocates");

    let mut cmd = CommandBuilder::new(env!("CARGO_BIN_EXE_saya"));
    cmd.env("SAYA_CONFIG_HOME", root.join("config-home"));
    cmd.env("SAYA_STATE_DB", root.join("state.sqlite3"));
    cmd.env("HOME", &root);
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd.env_remove("XDG_DATA_HOME");
    cmd.env_remove("APPDATA");
    cmd.cwd(&root);
    cmd.arg("setup");
    let mut child = pair
        .slave
        .spawn_command(cmd)
        .expect("child spawns on the pty");
    drop(pair.slave);

    // Feed the whole script up front: the flow reads line by line and the pty
    // buffers input, so no answer timing is needed. 6 = skip provider,
    // 1 = sqlite, path, profile name, y = confirm the write.
    let mut writer = pair.master.take_writer().unwrap();
    for line in ["6", "1", db_file.to_str().unwrap(), "team", "y"] {
        writer.write_all(line.as_bytes()).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
    }
    drop(writer);

    // Pump output on a reader thread; the main loop waits for the child to
    // exit and the stream to go quiet, with a hard deadline so CI never hangs.
    let reader = pair.master.try_clone_reader().unwrap();
    let (tx, rx) = std::sync::mpsc::channel::<Vec<u8>>();
    std::thread::spawn(move || {
        let mut reader = reader;
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut output = Vec::new();
    let mut quiet_since: Option<Instant> = None;
    loop {
        assert!(
            Instant::now() < deadline,
            "setup never finished on the pty; captured:\n{}",
            String::from_utf8_lossy(&output)
        );
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(chunk) => {
                output.extend_from_slice(&chunk);
                quiet_since = None;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                if child.try_wait().unwrap().is_some() {
                    let quiet = quiet_since.get_or_insert_with(Instant::now);
                    if quiet.elapsed() > Duration::from_millis(500) {
                        break;
                    }
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait().unwrap();
    let text = String::from_utf8_lossy(&output).into_owned();
    assert_eq!(status.exit_code(), 0, "the flow succeeds: {text}");
    assert!(
        text.contains("database reachable"),
        "the probe label: {text}"
    );
    assert!(
        text.contains("configuration valid"),
        "the commit label: {text}"
    );
    assert!(
        text.contains("saya --profile team"),
        "the next step: {text}"
    );

    let connections = root.join("config-home/saya/connections.toml");
    let written = fs::read_to_string(&connections).unwrap();
    assert!(
        written.contains("[profiles.team]"),
        "the profile is written: {written}"
    );
    assert!(
        written.contains(db_file.to_str().unwrap()),
        "the path is kept: {written}"
    );
    assert!(
        !root.join("config-home/saya/.setup-commit.json").exists(),
        "no marker is left"
    );
    let _ = fs::remove_dir_all(&root);
}
